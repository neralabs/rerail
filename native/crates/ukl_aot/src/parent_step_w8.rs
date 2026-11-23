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
        let t218 = b8_r0r1si_p(l, lid0, t216, arg0, arg1, (x16).wrapping_add(1u32), x24);
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
    let t219 = b4_r0r1s_f(l, lid0, arg0, arg1, x16);
    x32 = t219;
    let t220: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
    let t221: [uint32x4_t; 2] = [vshlq_u32(t220[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t220[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t222 = b5_r0r1sl_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x16), t221);
    x40 = t222;
    let t223 = b6_s_f(l, lid0, x32);
    x48 = t223;
    let t224 = [vdupq_n_u32(1023u32); 2];
    let t225: [uint32x4_t; 2] = [vandq_u32(t220[0], t224[0]), vandq_u32(t220[1], t224[1])];
    let t226 = [vdupq_n_u32(x32); 2];
    let t227: [uint32x4_t; 2] = [vmulq_u32(t225[0], t226[0]), vmulq_u32(t225[1], t226[1])];
    let t228 = b7_r0r1lls_f(l, lid0, arg0, arg1, x40, t227, 8u32);
    let t229: u64 = x48;
    let t230 = [vdupq_n_u32((t229) as u32); 2];
    let t231 = [vdupq_n_u32(((t229) >> 32) as u32); 2];
    let t232: [uint32x4_t; 4] = [vandq_u32(t228[0], t230[0]), vandq_u32(t228[1], t230[1]), vandq_u32(t228[2], t231[0]), vandq_u32(t228[3], t231[1])];
    x56 = t232;
    let t233: u64 = x48;
    let t234 = [vdupq_n_u32((t233) as u32); 2];
    let t235 = [vdupq_n_u32(((t233) >> 32) as u32); 2];
    let t236: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t234[0]), vceqq_u32(x56[2], t235[0])), vandq_u32(vceqq_u32(x56[1], t234[1]), vceqq_u32(x56[3], t235[1]))];
    let mut t237: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t238: [uint32x4_t; 2] = [vmvnq_u32(t236[0]), vmvnq_u32(t236[1])];
    if any(&t236) {
        let t239 = b8_r0r1si_p(l, lid0, t236, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t237 = [vbslq_u32(t236[0], t239[0], t237[0]), vbslq_u32(t236[1], t239[1], t237[1]), vbslq_u32(t236[0], t239[2], t237[2]), vbslq_u32(t236[1], t239[3], t237[3])];
    }
    {
        t237 = [vbslq_u32(t238[0], x56[0], t237[0]), vbslq_u32(t238[1], x56[1], t237[1]), vbslq_u32(t238[0], x56[2], t237[2]), vbslq_u32(t238[1], x56[3], t237[3])];
    }
    res = t237;
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
    let t240 = b4_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    x32 = t240;
    let t241 = b5_r0r1ss_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x16), (x24 >> 10));
    x40 = t241;
    let t242 = b6_s_p(l, lid0, live, x32);
    x48 = t242;
    let t243 = b7_r0r1sns_p(l, lid0, live, arg0, arg1, x40, (x24 & 1023u32).wrapping_mul(x32), x32, 8u32);
    let t244: u64 = x48;
    let t245 = [vdupq_n_u32((t244) as u32); 2];
    let t246 = [vdupq_n_u32(((t244) >> 32) as u32); 2];
    let t247: [uint32x4_t; 4] = [vandq_u32(t243[0], t245[0]), vandq_u32(t243[1], t245[1]), vandq_u32(t243[2], t246[0]), vandq_u32(t243[3], t246[1])];
    x56 = t247;
    let t248: u64 = x48;
    let t249 = [vdupq_n_u32((t248) as u32); 2];
    let t250 = [vdupq_n_u32(((t248) >> 32) as u32); 2];
    let t251: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t249[0]), vceqq_u32(x56[2], t250[0])), vandq_u32(vceqq_u32(x56[1], t249[1]), vceqq_u32(x56[3], t250[1]))];
    let mut t252: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t253: [uint32x4_t; 2] = [vandq_u32(live[0], t251[0]), vandq_u32(live[1], t251[1])];
    let t254: [uint32x4_t; 2] = [vbicq_u32(live[0], t251[0]), vbicq_u32(live[1], t251[1])];
    if any(&t253) {
        let t255 = b8_r0r1su8_p(l, lid0, t253, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t252 = [vbslq_u32(t253[0], t255[0], t252[0]), vbslq_u32(t253[1], t255[1], t252[1]), vbslq_u32(t253[0], t255[2], t252[2]), vbslq_u32(t253[1], t255[3], t252[3])];
    }
    {
        t252 = [vbslq_u32(t254[0], x56[0], t252[0]), vbslq_u32(t254[1], x56[1], t252[1]), vbslq_u32(t254[0], x56[2], t252[2]), vbslq_u32(t254[1], x56[3], t252[3])];
    }
    res = t252;
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
    let t256 = b4_r0r1s_f(l, lid0, arg0, arg1, x16);
    x32 = t256;
    let t257 = b5_r0r1ss_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x16), (x24 >> 10));
    x40 = t257;
    let t258 = b6_s_f(l, lid0, x32);
    x48 = t258;
    let t259 = b7_r0r1sns_f(l, lid0, arg0, arg1, x40, (x24 & 1023u32).wrapping_mul(x32), x32, 8u32);
    let t260: u64 = x48;
    let t261 = [vdupq_n_u32((t260) as u32); 2];
    let t262 = [vdupq_n_u32(((t260) >> 32) as u32); 2];
    let t263: [uint32x4_t; 4] = [vandq_u32(t259[0], t261[0]), vandq_u32(t259[1], t261[1]), vandq_u32(t259[2], t262[0]), vandq_u32(t259[3], t262[1])];
    x56 = t263;
    let t264: u64 = x48;
    let t265 = [vdupq_n_u32((t264) as u32); 2];
    let t266 = [vdupq_n_u32(((t264) >> 32) as u32); 2];
    let t267: [uint32x4_t; 2] = [vandq_u32(vceqq_u32(x56[0], t265[0]), vceqq_u32(x56[2], t266[0])), vandq_u32(vceqq_u32(x56[1], t265[1]), vceqq_u32(x56[3], t266[1]))];
    let mut t268: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t269: [uint32x4_t; 2] = [vmvnq_u32(t267[0]), vmvnq_u32(t267[1])];
    if any(&t267) {
        let t270 = b8_r0r1su8_p(l, lid0, t267, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t268 = [vbslq_u32(t267[0], t270[0], t268[0]), vbslq_u32(t267[1], t270[1], t268[1]), vbslq_u32(t267[0], t270[2], t268[2]), vbslq_u32(t267[1], t270[3], t268[3])];
    }
    {
        t268 = [vbslq_u32(t269[0], x56[0], t268[0]), vbslq_u32(t269[1], x56[1], t268[1]), vbslq_u32(t269[0], x56[2], t268[2]), vbslq_u32(t269[1], x56[3], t268[3])];
    }
    res = t268;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t271 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    res = (((t271).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t272 = b9_r0r1s_f(l, lid0, arg0, arg1, 16u32);
    res = (((t272).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: u32 = 0;
    let mut live = k;
    let mut res: u32 = 0;
    let t273 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t274 = b10_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    x32 = ((x24).wrapping_mul(t273)).wrapping_add(t274);
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t275 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t276 = b11_ss_p(l, lid0, live, (t275).wrapping_sub((x24).wrapping_shl((10u32) as u32)), 1024u32);
    x40 = t276;
    let t277 = b10_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    res = (x32).wrapping_add((x40).wrapping_mul(t277));
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1ss_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: u32 = 0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t278 = b9_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t279 = b10_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    x32 = ((x24).wrapping_mul(t278)).wrapping_add(t279);
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t280 = b9_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t281 = b11_ss_f(l, lid0, (t280).wrapping_sub((x24).wrapping_shl((10u32) as u32)), 1024u32);
    x40 = t281;
    let t282 = b10_r0r1s_f(l, lid0, arg0, arg1, x16);
    res = (x32).wrapping_add((x40).wrapping_mul(t282));
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t283 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t284 = [vdupq_n_u32(t283); 2];
    let t285: [uint32x4_t; 2] = [vmulq_u32(x24[0], t284[0]), vmulq_u32(x24[1], t284[1])];
    let t286 = b10_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    let t287 = [vdupq_n_u32(t286); 2];
    let t288: [uint32x4_t; 2] = [vaddq_u32(t285[0], t287[0]), vaddq_u32(t285[1], t287[1])];
    x32 = t288;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t289 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t290: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t291 = [vdupq_n_u32(t289); 2];
    let t292: [uint32x4_t; 2] = [vsubq_u32(t291[0], t290[0]), vsubq_u32(t291[1], t290[1])];
    let t293 = b11_ls_p(l, lid0, live, t292, 1024u32);
    x40 = t293;
    let t294 = b10_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    let t295 = [vdupq_n_u32(t294); 2];
    let t296: [uint32x4_t; 2] = [vmulq_u32(x40[0], t295[0]), vmulq_u32(x40[1], t295[1])];
    let t297: [uint32x4_t; 2] = [vaddq_u32(x32[0], t296[0]), vaddq_u32(x32[1], t296[1])];
    res = t297;
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1sl_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t298 = b9_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t299 = [vdupq_n_u32(t298); 2];
    let t300: [uint32x4_t; 2] = [vmulq_u32(x24[0], t299[0]), vmulq_u32(x24[1], t299[1])];
    let t301 = b10_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    let t302 = [vdupq_n_u32(t301); 2];
    let t303: [uint32x4_t; 2] = [vaddq_u32(t300[0], t302[0]), vaddq_u32(t300[1], t302[1])];
    x32 = t303;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t304 = b9_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t305: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t306 = [vdupq_n_u32(t304); 2];
    let t307: [uint32x4_t; 2] = [vsubq_u32(t306[0], t305[0]), vsubq_u32(t306[1], t305[1])];
    let t308 = b11_ls_f(l, lid0, t307, 1024u32);
    x40 = t308;
    let t309 = b10_r0r1s_f(l, lid0, arg0, arg1, x16);
    let t310 = [vdupq_n_u32(t309); 2];
    let t311: [uint32x4_t; 2] = [vmulq_u32(x40[0], t310[0]), vmulq_u32(x40[1], t310[1])];
    let t312: [uint32x4_t; 2] = [vaddq_u32(x32[0], t311[0]), vaddq_u32(x32[1], t311[1])];
    res = t312;
    return res;
}

#[inline(always)]
unsafe fn b6_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t313: u64;
    if ((x0) >= (8u32)) {
        t313 = !(0u64);
    } else {
        t313 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t313;
    return res;
}

#[inline(always)]
unsafe fn b6_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t314: u64;
    if ((x0) >= (8u32)) {
        t314 = !(0u64);
    } else {
        t314 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t314;
    return res;
}

#[inline(always)]
unsafe fn b6_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2]) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t315 = [vdupq_n_u32(8u32); 2];
    let t316: [uint32x4_t; 2] = [vcgeq_u32(x0[0], t315[0]), vcgeq_u32(x0[1], t315[1])];
    let mut t317: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t318: [uint32x4_t; 2] = [vandq_u32(live[0], t316[0]), vandq_u32(live[1], t316[1])];
    let t319: [uint32x4_t; 2] = [vbicq_u32(live[0], t316[0]), vbicq_u32(live[1], t316[1])];
    {
        let t320: u64 = !(0u64);
        let t321 = [vdupq_n_u32((t320) as u32); 2];
        let t322 = [vdupq_n_u32(((t320) >> 32) as u32); 2];
        let t323: [uint32x4_t; 4] = [t321[0], t321[1], t322[0], t322[1]];
        t317 = [vbslq_u32(t318[0], t323[0], t317[0]), vbslq_u32(t318[1], t323[1], t317[1]), vbslq_u32(t318[0], t323[2], t317[2]), vbslq_u32(t318[1], t323[3], t317[3])];
    }
    {
        let t324: [uint32x4_t; 2] = [vmulq_u32(x0[0], t315[0]), vmulq_u32(x0[1], t315[1])];
        let t325: u64 = 1u64;
        let t326 = [vdupq_n_u32((t325) as u32); 2];
        let t327: [uint32x4_t; 2] = [t324[0], t324[1]];
        let t328: [uint32x4_t; 2] = [vandq_u32(t327[0], vdupq_n_u32(63)), vandq_u32(t327[1], vdupq_n_u32(63))];
        let t329: [uint32x4_t; 4] = [vshlq_u32(t326[0], vreinterpretq_s32_u32(t328[0])), vshlq_u32(t326[1], vreinterpretq_s32_u32(t328[1])), vshlq_u32(t326[0], vsubq_s32(vreinterpretq_s32_u32(t328[0]), vdupq_n_s32(32))), vshlq_u32(t326[1], vsubq_s32(vreinterpretq_s32_u32(t328[1]), vdupq_n_s32(32)))];
        let t330: u64 = 1u64;
        let t331 = [vdupq_n_u32((t330) as u32); 2];
        let t332: [uint32x4_t; 2] = [vcltq_u32(t329[0], t331[0]), vcltq_u32(t329[1], t331[1])];
        let t333: [uint32x4_t; 4] = [vsubq_u32(t329[0], t331[0]), vsubq_u32(t329[1], t331[1]), vaddq_u32(vsubq_u32(t329[2], vdupq_n_u32(0)), t332[0]), vaddq_u32(vsubq_u32(t329[3], vdupq_n_u32(0)), t332[1])];
        t317 = [vbslq_u32(t319[0], t333[0], t317[0]), vbslq_u32(t319[1], t333[1], t317[1]), vbslq_u32(t319[0], t333[2], t317[2]), vbslq_u32(t319[1], t333[3], t317[3])];
    }
    res = t317;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1sns_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24f: u32, x24s: u32, x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t334 = b12_r0sns_p(l, lid0, live, arg0, x16, x24f, x24s, x32);
    res = t334;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1sns_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24f: u32, x24s: u32, x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t335 = b12_r0sns_f(l, lid0, arg0, x16, x24f, x24s, x32);
    res = t335;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t336 = b12_r0lls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t336;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1lls_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t337 = b12_r0lls_f(l, lid0, arg0, x16, x24, x32);
    res = t337;
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
    let t338 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t338;
    let t339 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t339;
    let t340: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x24), vld1q_u32(LANE[1].as_ptr()))];
    let t341: [uint32x4_t; 2] = [vshlq_u32(t340[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(t340[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x48 = t341;
    let t342 = [vdupq_n_u32(4u32); 2];
    let t343: [uint32x4_t; 2] = [vmulq_u32(x48[0], t342[0]), vmulq_u32(x48[1], t342[1])];
    let t344 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t343, 4u32);
    let t345: [uint32x4_t; 2] = [t344[0], t344[1]];
    x56 = t345;
    let t346: [uint32x4_t; 2] = [vmulq_u32(x48[0], t342[0]), vmulq_u32(x48[1], t342[1])];
    let t347: [uint32x4_t; 2] = [vaddq_u32(t346[0], t342[0]), vaddq_u32(t346[1], t342[1])];
    let t348 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t347, 4u32);
    let t349: [uint32x4_t; 2] = [t348[0], t348[1]];
    x64 = t349;
    let t350 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x72 = ((t350).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t351: [uint32x4_t; 2] = [vmulq_u32(x48[0], t342[0]), vmulq_u32(x48[1], t342[1])];
    let t352 = [vdupq_n_u32(x72); 2];
    let t353: [uint32x4_t; 2] = [vsubq_u32(t352[0], t351[0]), vsubq_u32(t352[1], t351[1])];
    let t354 = b14_sl_p(l, lid0, live, 4u32, t353);
    x80 = t354;
    let t355: [uint32x4_t; 2] = [vshlq_u32(t340[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(t340[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t356 = [vdupq_n_u32(3u32); 2];
    let t357: [uint32x4_t; 2] = [vandq_u32(t355[0], t356[0]), vandq_u32(t355[1], t356[1])];
    x88 = t357;
    let t358 = [vdupq_n_u32(0u32); 2];
    let t359: [uint32x4_t; 2] = [vceqq_u32(x88[0], t358[0]), vceqq_u32(x88[1], t358[1])];
    let mut t360: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t361: [uint32x4_t; 2] = [vandq_u32(live[0], t359[0]), vandq_u32(live[1], t359[1])];
    let t362: [uint32x4_t; 2] = [vbicq_u32(live[0], t359[0]), vbicq_u32(live[1], t359[1])];
    {
        t360 = [vbslq_u32(t361[0], t358[0], t360[0]), vbslq_u32(t361[1], t358[1], t360[1])];
    }
    if any(&t362) {
        let t363 = [vdupq_n_u32(1u32); 2];
        let t364: [uint32x4_t; 2] = [vsubq_u32(x88[0], t363[0]), vsubq_u32(x88[1], t363[1])];
        let t365 = [vdupq_n_u32(2u32); 2];
        let t366: [uint32x4_t; 2] = [vmulq_u32(t365[0], t364[0]), vmulq_u32(t365[1], t364[1])];
        let t367: [uint32x4_t; 2] = [vaddq_u32(x56[0], t366[0]), vaddq_u32(x56[1], t366[1])];
        let t368 = b15_ll_p(l, lid0, t362, t367, x64);
        let t369 = b13_r0r1sls_p(l, lid0, t362, arg0, arg1, x32, t368, 2u32);
        let t370: [uint32x4_t; 2] = [t369[0], t369[1]];
        t360 = [vbslq_u32(t362[0], t370[0], t360[0]), vbslq_u32(t362[1], t370[1], t360[1])];
    }
    x96 = t360;
    let t371 = [vdupq_n_u32(2u32); 2];
    let t372: [uint32x4_t; 2] = [vmulq_u32(t371[0], x88[0]), vmulq_u32(t371[1], x88[1])];
    let t373: [uint32x4_t; 2] = [vaddq_u32(x56[0], t372[0]), vaddq_u32(x56[1], t372[1])];
    let t374 = b15_ll_p(l, lid0, live, t373, x64);
    let t375 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t374, 2u32);
    let t376: [uint32x4_t; 2] = [t375[0], t375[1]];
    x104 = t376;
    let t377: [uint32x4_t; 2] = [vmulq_u32(t371[0], x80[0]), vmulq_u32(t371[1], x80[1])];
    let t378: [uint32x4_t; 2] = [vaddq_u32(x56[0], t377[0]), vaddq_u32(x56[1], t377[1])];
    x112 = t378;
    let t379 = [vdupq_n_u32(255u32); 2];
    let t380: [uint32x4_t; 2] = [vandq_u32(t340[0], t379[0]), vandq_u32(t340[1], t379[1])];
    x120 = t380;
    x128 = 9259542123273814144u64;
    let t381: u64 = 72340172838076673u64;
    let t382 = [vdupq_n_u32((t381) as u32); 2];
    let t383 = [vdupq_n_u32(((t381) >> 32) as u32); 2];
    let t384: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x120[0]), vget_low_u32(t382[0]))), vreinterpretq_u32_u64(vmull_high_u32(x120[0], t382[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(x120[1]), vget_low_u32(t382[1]))), vreinterpretq_u32_u64(vmull_high_u32(x120[1], t382[1])))];
    let t385: [uint32x4_t; 4] = [vmulq_u32(x120[0], t382[0]), vmulq_u32(x120[1], t382[1]), vmlaq_u32(t384[0], x120[0], t383[0]), vmlaq_u32(t384[1], x120[1], t383[1])];
    x136 = t385;
    x144 = x96;
    x152 = x96;
    let mut t386 = live;
    loop {
        // Lanes part: each its own rounds.
        let t388: [uint32x4_t; 2] = [vcltq_u32(x152[0], x104[0]), vcltq_u32(x152[1], x104[1])];
        t386 = [vandq_u32(t386[0], t388[0]), vandq_u32(t386[1], t388[1])];
        if !(any(&t386)) { break; }
        let t390: [uint32x4_t; 2] = [vaddq_u32(x112[0], x152[0]), vaddq_u32(x112[1], x152[1])];
        let t391 = b15_ll_p(l, lid0, t386, t390, x64);
        let t392 = b13_r0r1sls_p(l, lid0, t386, arg0, arg1, x32, t391, 8u32);
        x160 = [vbslq_u32(t386[0], t392[0], x160[0]), vbslq_u32(t386[1], t392[1], x160[1]), vbslq_u32(t386[0], t392[2], x160[2]), vbslq_u32(t386[1], t392[3], x160[3])];
        let t393: u64 = x128;
        let t394 = [vdupq_n_u32((t393) as u32); 2];
        let t395 = [vdupq_n_u32(((t393) >> 32) as u32); 2];
        let t396: [uint32x4_t; 4] = [vorrq_u32(x160[0], t394[0]), vorrq_u32(x160[1], t394[1]), vorrq_u32(x160[2], t395[0]), vorrq_u32(x160[3], t395[1])];
        let t397: u64 = !(x128);
        let t398 = [vdupq_n_u32((t397) as u32); 2];
        let t399 = [vdupq_n_u32(((t397) >> 32) as u32); 2];
        let t400: [uint32x4_t; 4] = [vandq_u32(x136[0], t398[0]), vandq_u32(x136[1], t398[1]), vandq_u32(x136[2], t399[0]), vandq_u32(x136[3], t399[1])];
        let t401: [uint32x4_t; 2] = [vcltq_u32(t396[0], t400[0]), vcltq_u32(t396[1], t400[1])];
        let t402: [uint32x4_t; 4] = [vsubq_u32(t396[0], t400[0]), vsubq_u32(t396[1], t400[1]), vaddq_u32(vsubq_u32(t396[2], t400[2]), t401[0]), vaddq_u32(vsubq_u32(t396[3], t400[3]), t401[1])];
        x168 = [vbslq_u32(t386[0], t402[0], x168[0]), vbslq_u32(t386[1], t402[1], x168[1]), vbslq_u32(t386[0], t402[2], x168[2]), vbslq_u32(t386[1], t402[3], x168[3])];
        let t403: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x160[r]));
        let t404: [uint32x4_t; 4] = [vandq_u32(t403[0], x136[0]), vandq_u32(t403[1], x136[1]), vandq_u32(t403[2], x136[2]), vandq_u32(t403[3], x136[3])];
        let t405: [uint32x4_t; 4] = [veorq_u32(x160[0], x136[0]), veorq_u32(x160[1], x136[1]), veorq_u32(x160[2], x136[2]), veorq_u32(x160[3], x136[3])];
        let t406: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t405[r]));
        let t407: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x168[r]));
        let t408: [uint32x4_t; 4] = [vandq_u32(t406[0], t407[0]), vandq_u32(t406[1], t407[1]), vandq_u32(t406[2], t407[2]), vandq_u32(t406[3], t407[3])];
        let t409: [uint32x4_t; 4] = [vorrq_u32(t404[0], t408[0]), vorrq_u32(t404[1], t408[1]), vorrq_u32(t404[2], t408[2]), vorrq_u32(t404[3], t408[3])];
        let t410: u64 = x128;
        let t411 = [vdupq_n_u32((t410) as u32); 2];
        let t412 = [vdupq_n_u32(((t410) >> 32) as u32); 2];
        let t413: [uint32x4_t; 4] = [vandq_u32(t411[0], t409[0]), vandq_u32(t411[1], t409[1]), vandq_u32(t412[0], t409[2]), vandq_u32(t412[1], t409[3])];
        x176 = t413;
        let t414: [uint32x4_t; 2] = [vsubq_u32(x104[0], x152[0]), vsubq_u32(x104[1], x152[1])];
        x184 = t414;
        let t415 = [vdupq_n_u32(8u32); 2];
        let t416: [uint32x4_t; 2] = [vcgeq_u32(x184[0], t415[0]), vcgeq_u32(x184[1], t415[1])];
        let mut t417: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t418: [uint32x4_t; 2] = [vandq_u32(t386[0], t416[0]), vandq_u32(t386[1], t416[1])];
        let t419: [uint32x4_t; 2] = [vbicq_u32(t386[0], t416[0]), vbicq_u32(t386[1], t416[1])];
        {
            t417 = [vbslq_u32(t418[0], x176[0], t417[0]), vbslq_u32(t418[1], x176[1], t417[1]), vbslq_u32(t418[0], x176[2], t417[2]), vbslq_u32(t418[1], x176[3], t417[3])];
        }
        {
            let t420: [uint32x4_t; 2] = [vmulq_u32(t415[0], x184[0]), vmulq_u32(t415[1], x184[1])];
            let t421: u64 = 1u64;
            let t422 = [vdupq_n_u32((t421) as u32); 2];
            let t423: [uint32x4_t; 2] = [t420[0], t420[1]];
            let t424: [uint32x4_t; 2] = [vandq_u32(t423[0], vdupq_n_u32(63)), vandq_u32(t423[1], vdupq_n_u32(63))];
            let t425: [uint32x4_t; 4] = [vshlq_u32(t422[0], vreinterpretq_s32_u32(t424[0])), vshlq_u32(t422[1], vreinterpretq_s32_u32(t424[1])), vshlq_u32(t422[0], vsubq_s32(vreinterpretq_s32_u32(t424[0]), vdupq_n_s32(32))), vshlq_u32(t422[1], vsubq_s32(vreinterpretq_s32_u32(t424[1]), vdupq_n_s32(32)))];
            let t426: u64 = 1u64;
            let t427 = [vdupq_n_u32((t426) as u32); 2];
            let t428: [uint32x4_t; 2] = [vcltq_u32(t425[0], t427[0]), vcltq_u32(t425[1], t427[1])];
            let t429: [uint32x4_t; 4] = [vsubq_u32(t425[0], t427[0]), vsubq_u32(t425[1], t427[1]), vaddq_u32(vsubq_u32(t425[2], vdupq_n_u32(0)), t428[0]), vaddq_u32(vsubq_u32(t425[3], vdupq_n_u32(0)), t428[1])];
            let t430: [uint32x4_t; 4] = [vandq_u32(x176[0], t429[0]), vandq_u32(x176[1], t429[1]), vandq_u32(x176[2], t429[2]), vandq_u32(x176[3], t429[3])];
            t417 = [vbslq_u32(t419[0], t430[0], t417[0]), vbslq_u32(t419[1], t430[1], t417[1]), vbslq_u32(t419[0], t430[2], t417[2]), vbslq_u32(t419[1], t430[3], t417[3])];
        }
        let t431 = b16_l_p(l, lid0, t386, t417);
        let t432: [uint32x4_t; 2] = [vaddq_u32(x144[0], t431[0]), vaddq_u32(x144[1], t431[1])];
        x144 = [vbslq_u32(t386[0], t432[0], x144[0]), vbslq_u32(t386[1], t432[1], x144[1])];
        let t433: [uint32x4_t; 2] = [vandq_u32(t386[0], live[0]), vandq_u32(t386[1], live[1])];
        x152 = [vaddq_u32(x152[0], vandq_u32(t433[0], vdupq_n_u32(8))), vaddq_u32(x152[1], vandq_u32(t433[1], vdupq_n_u32(8)))];
    }
    x96 = x144;
    let t434 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    let t435 = [vdupq_n_u32((((t434).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32))); 2];
    x152 = t435;
    let t436 = [vdupq_n_u32(1u32); 2];
    let t437: [uint32x4_t; 2] = [vsubq_u32(x80[0], t436[0]), vsubq_u32(x80[1], t436[1])];
    let t438: [uint32x4_t; 2] = [vmulq_u32(t371[0], t437[0]), vmulq_u32(t371[1], t437[1])];
    let t439: [uint32x4_t; 2] = [vaddq_u32(x56[0], t438[0]), vaddq_u32(x56[1], t438[1])];
    let t440 = b15_ll_p(l, lid0, live, t439, x64);
    let t441 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t440, 2u32);
    let t442: [uint32x4_t; 2] = [t441[0], t441[1]];
    let t443: [uint32x4_t; 4] = [t442[0], t442[1], vdupq_n_u32(0), vdupq_n_u32(0)];
    x160 = t443;
    let t444: [uint32x4_t; 2] = [x160[0], x160[1]];
    let t445: [uint32x4_t; 2] = [vaddq_u32(x112[0], t444[0]), vaddq_u32(x112[1], t444[1])];
    let t446: [uint32x4_t; 2] = [vmulq_u32(x96[0], x152[0]), vmulq_u32(x96[1], x152[1])];
    let t447: [uint32x4_t; 2] = [vaddq_u32(t445[0], t446[0]), vaddq_u32(t445[1], t446[1])];
    let t448 = b15_ll_p(l, lid0, live, t447, x64);
    let t449: [uint32x4_t; 4] = [t448[0], t448[1], vdupq_n_u32(0), vdupq_n_u32(0)];
    x168 = t449;
    let t450: [uint32x4_t; 2] = [x168[0], x168[1]];
    let t451 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t450, 8u32);
    let t452 = b6_l_p(l, lid0, live, x152);
    let t453: [uint32x4_t; 4] = [vandq_u32(t451[0], t452[0]), vandq_u32(t451[1], t452[1]), vandq_u32(t451[2], t452[2]), vandq_u32(t451[3], t452[3])];
    res = t453;
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
    let t454 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t454;
    let t455 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t455;
    x48 = (x24 >> 10);
    let t456 = b13_r0r1sss_p(l, lid0, live, arg0, arg1, x40, (x48).wrapping_mul(4u32), 4u32);
    x56 = ((t456) as u32);
    let t457 = b13_r0r1sss_p(l, lid0, live, arg0, arg1, x40, ((x48).wrapping_mul(4u32)).wrapping_add(4u32), 4u32);
    x64 = ((t457) as u32);
    let t458 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x72 = ((t458).wrapping_add(255u32)).wrapping_shr((8u32) as u32);
    let t459 = b14_ss_p(l, lid0, live, 4u32, (x72).wrapping_sub((x48).wrapping_mul(4u32)));
    x80 = t459;
    x88 = (((x24 >> 8)) & (3u32));
    let t460: u32;
    if ((x88) == (0u32)) {
        t460 = 0u32;
    } else {
        let t461 = b15_ss_p(l, lid0, live, (x56).wrapping_add((2u32).wrapping_mul((x88).wrapping_sub(1u32))), x64);
        let t462 = b13_r0r1sss_p(l, lid0, live, arg0, arg1, x32, t461, 2u32);
        t460 = ((t462) as u32);
    }
    let t463 = [vdupq_n_u32(t460); 2];
    x96 = t463;
    let t464 = b15_ss_p(l, lid0, live, (x56).wrapping_add((2u32).wrapping_mul(x88)), x64);
    let t465 = b13_r0r1sss_p(l, lid0, live, arg0, arg1, x32, t464, 2u32);
    x104 = ((t465) as u32);
    x112 = (x56).wrapping_add((2u32).wrapping_mul(x80));
    x120 = (x24 & 255u32);
    x128 = 9259542123273814144u64;
    let t466: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x120), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x120), vld1q_u32(LANE[1].as_ptr()))];
    let t467: u64 = 72340172838076673u64;
    let t468 = [vdupq_n_u32((t467) as u32); 2];
    let t469 = [vdupq_n_u32(((t467) >> 32) as u32); 2];
    let t470: [uint32x4_t; 2] = [vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t466[0]), vget_low_u32(t468[0]))), vreinterpretq_u32_u64(vmull_high_u32(t466[0], t468[0]))), vuzp2q_u32(vreinterpretq_u32_u64(vmull_u32(vget_low_u32(t466[1]), vget_low_u32(t468[1]))), vreinterpretq_u32_u64(vmull_high_u32(t466[1], t468[1])))];
    let t471: [uint32x4_t; 4] = [vmulq_u32(t466[0], t468[0]), vmulq_u32(t466[1], t468[1]), vmlaq_u32(t470[0], t466[0], t469[0]), vmlaq_u32(t470[1], t466[1], t469[1])];
    x136 = t471;
    x144 = x96;
    x152 = x96;
    let mut t472 = live;
    loop {
        // Lanes part: each its own rounds.
        let t474 = [vdupq_n_u32(x104); 2];
        let t475: [uint32x4_t; 2] = [vcltq_u32(x152[0], t474[0]), vcltq_u32(x152[1], t474[1])];
        t472 = [vandq_u32(t472[0], t475[0]), vandq_u32(t472[1], t475[1])];
        if !(any(&t472)) { break; }
        let t477 = [vdupq_n_u32(x112); 2];
        let t478: [uint32x4_t; 2] = [vaddq_u32(t477[0], x152[0]), vaddq_u32(t477[1], x152[1])];
        let t479 = b15_ls_p(l, lid0, t472, t478, x64);
        let t480 = b13_r0r1sls_p(l, lid0, t472, arg0, arg1, x32, t479, 8u32);
        x160 = [vbslq_u32(t472[0], t480[0], x160[0]), vbslq_u32(t472[1], t480[1], x160[1]), vbslq_u32(t472[0], t480[2], x160[2]), vbslq_u32(t472[1], t480[3], x160[3])];
        let t481: u64 = x128;
        let t482 = [vdupq_n_u32((t481) as u32); 2];
        let t483 = [vdupq_n_u32(((t481) >> 32) as u32); 2];
        let t484: [uint32x4_t; 4] = [vorrq_u32(x160[0], t482[0]), vorrq_u32(x160[1], t482[1]), vorrq_u32(x160[2], t483[0]), vorrq_u32(x160[3], t483[1])];
        let t485: u64 = !(x128);
        let t486 = [vdupq_n_u32((t485) as u32); 2];
        let t487 = [vdupq_n_u32(((t485) >> 32) as u32); 2];
        let t488: [uint32x4_t; 4] = [vandq_u32(x136[0], t486[0]), vandq_u32(x136[1], t486[1]), vandq_u32(x136[2], t487[0]), vandq_u32(x136[3], t487[1])];
        let t489: [uint32x4_t; 2] = [vcltq_u32(t484[0], t488[0]), vcltq_u32(t484[1], t488[1])];
        let t490: [uint32x4_t; 4] = [vsubq_u32(t484[0], t488[0]), vsubq_u32(t484[1], t488[1]), vaddq_u32(vsubq_u32(t484[2], t488[2]), t489[0]), vaddq_u32(vsubq_u32(t484[3], t488[3]), t489[1])];
        x168 = [vbslq_u32(t472[0], t490[0], x168[0]), vbslq_u32(t472[1], t490[1], x168[1]), vbslq_u32(t472[0], t490[2], x168[2]), vbslq_u32(t472[1], t490[3], x168[3])];
        let t491: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x160[r]));
        let t492: [uint32x4_t; 4] = [vandq_u32(t491[0], x136[0]), vandq_u32(t491[1], x136[1]), vandq_u32(t491[2], x136[2]), vandq_u32(t491[3], x136[3])];
        let t493: [uint32x4_t; 4] = [veorq_u32(x160[0], x136[0]), veorq_u32(x160[1], x136[1]), veorq_u32(x160[2], x136[2]), veorq_u32(x160[3], x136[3])];
        let t494: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(t493[r]));
        let t495: [uint32x4_t; 4] = core::array::from_fn(|r| vmvnq_u32(x168[r]));
        let t496: [uint32x4_t; 4] = [vandq_u32(t494[0], t495[0]), vandq_u32(t494[1], t495[1]), vandq_u32(t494[2], t495[2]), vandq_u32(t494[3], t495[3])];
        let t497: [uint32x4_t; 4] = [vorrq_u32(t492[0], t496[0]), vorrq_u32(t492[1], t496[1]), vorrq_u32(t492[2], t496[2]), vorrq_u32(t492[3], t496[3])];
        let t498: u64 = x128;
        let t499 = [vdupq_n_u32((t498) as u32); 2];
        let t500 = [vdupq_n_u32(((t498) >> 32) as u32); 2];
        let t501: [uint32x4_t; 4] = [vandq_u32(t499[0], t497[0]), vandq_u32(t499[1], t497[1]), vandq_u32(t500[0], t497[2]), vandq_u32(t500[1], t497[3])];
        x176 = t501;
        let t502 = [vdupq_n_u32(x104); 2];
        let t503: [uint32x4_t; 2] = [vsubq_u32(t502[0], x152[0]), vsubq_u32(t502[1], x152[1])];
        x184 = t503;
        let t504 = [vdupq_n_u32(8u32); 2];
        let t505: [uint32x4_t; 2] = [vcgeq_u32(x184[0], t504[0]), vcgeq_u32(x184[1], t504[1])];
        let mut t506: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        let t507: [uint32x4_t; 2] = [vandq_u32(t472[0], t505[0]), vandq_u32(t472[1], t505[1])];
        let t508: [uint32x4_t; 2] = [vbicq_u32(t472[0], t505[0]), vbicq_u32(t472[1], t505[1])];
        {
            t506 = [vbslq_u32(t507[0], x176[0], t506[0]), vbslq_u32(t507[1], x176[1], t506[1]), vbslq_u32(t507[0], x176[2], t506[2]), vbslq_u32(t507[1], x176[3], t506[3])];
        }
        {
            let t509: [uint32x4_t; 2] = [vmulq_u32(t504[0], x184[0]), vmulq_u32(t504[1], x184[1])];
            let t510: u64 = 1u64;
            let t511 = [vdupq_n_u32((t510) as u32); 2];
            let t512: [uint32x4_t; 2] = [t509[0], t509[1]];
            let t513: [uint32x4_t; 2] = [vandq_u32(t512[0], vdupq_n_u32(63)), vandq_u32(t512[1], vdupq_n_u32(63))];
            let t514: [uint32x4_t; 4] = [vshlq_u32(t511[0], vreinterpretq_s32_u32(t513[0])), vshlq_u32(t511[1], vreinterpretq_s32_u32(t513[1])), vshlq_u32(t511[0], vsubq_s32(vreinterpretq_s32_u32(t513[0]), vdupq_n_s32(32))), vshlq_u32(t511[1], vsubq_s32(vreinterpretq_s32_u32(t513[1]), vdupq_n_s32(32)))];
            let t515: u64 = 1u64;
            let t516 = [vdupq_n_u32((t515) as u32); 2];
            let t517: [uint32x4_t; 2] = [vcltq_u32(t514[0], t516[0]), vcltq_u32(t514[1], t516[1])];
            let t518: [uint32x4_t; 4] = [vsubq_u32(t514[0], t516[0]), vsubq_u32(t514[1], t516[1]), vaddq_u32(vsubq_u32(t514[2], vdupq_n_u32(0)), t517[0]), vaddq_u32(vsubq_u32(t514[3], vdupq_n_u32(0)), t517[1])];
            let t519: [uint32x4_t; 4] = [vandq_u32(x176[0], t518[0]), vandq_u32(x176[1], t518[1]), vandq_u32(x176[2], t518[2]), vandq_u32(x176[3], t518[3])];
            t506 = [vbslq_u32(t508[0], t519[0], t506[0]), vbslq_u32(t508[1], t519[1], t506[1]), vbslq_u32(t508[0], t519[2], t506[2]), vbslq_u32(t508[1], t519[3], t506[3])];
        }
        let t520 = b16_l_p(l, lid0, t472, t506);
        let t521: [uint32x4_t; 2] = [vaddq_u32(x144[0], t520[0]), vaddq_u32(x144[1], t520[1])];
        x144 = [vbslq_u32(t472[0], t521[0], x144[0]), vbslq_u32(t472[1], t521[1], x144[1])];
        let t522: [uint32x4_t; 2] = [vandq_u32(t472[0], live[0]), vandq_u32(t472[1], live[1])];
        x152 = [vaddq_u32(x152[0], vandq_u32(t522[0], vdupq_n_u32(8))), vaddq_u32(x152[1], vandq_u32(t522[1], vdupq_n_u32(8)))];
    }
    x96 = x144;
    let t523 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    let t524 = [vdupq_n_u32((((t523).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32))); 2];
    x152 = t524;
    let t525 = b15_ss_p(l, lid0, live, (x56).wrapping_add((2u32).wrapping_mul((x80).wrapping_sub(1u32))), x64);
    let t526 = b13_r0r1sss_p(l, lid0, live, arg0, arg1, x32, t525, 2u32);
    let t527: u64 = ((((t526) as u32)) as u64);
    let t528 = [vdupq_n_u32((t527) as u32); 2];
    let t529 = [vdupq_n_u32(((t527) >> 32) as u32); 2];
    let t530: [uint32x4_t; 4] = [t528[0], t528[1], t529[0], t529[1]];
    x160 = t530;
    let t531 = [vdupq_n_u32(x112); 2];
    let t532: [uint32x4_t; 2] = [x160[0], x160[1]];
    let t533: [uint32x4_t; 2] = [vaddq_u32(t531[0], t532[0]), vaddq_u32(t531[1], t532[1])];
    let t534: [uint32x4_t; 2] = [vmulq_u32(x96[0], x152[0]), vmulq_u32(x96[1], x152[1])];
    let t535: [uint32x4_t; 2] = [vaddq_u32(t533[0], t534[0]), vaddq_u32(t533[1], t534[1])];
    let t536 = b15_ls_p(l, lid0, live, t535, x64);
    let t537: [uint32x4_t; 4] = [t536[0], t536[1], vdupq_n_u32(0), vdupq_n_u32(0)];
    x168 = t537;
    let t538: [uint32x4_t; 2] = [x168[0], x168[1]];
    let t539 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t538, 8u32);
    let t540 = b6_l_p(l, lid0, live, x152);
    let t541: [uint32x4_t; 4] = [vandq_u32(t539[0], t540[0]), vandq_u32(t539[1], t540[1]), vandq_u32(t539[2], t540[2]), vandq_u32(t539[3], t540[3])];
    res = t541;
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
    let t542 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t542;
    return res;
}

#[inline(always)]
unsafe fn b10_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t543 = b9_r0r1s_f(l, lid0, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t543;
    return res;
}

#[inline(always)]
unsafe fn b11_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t544: u32;
    if ((x0) < (x8)) {
        t544 = x0;
    } else {
        t544 = x8;
    }
    res = t544;
    return res;
}

#[inline(always)]
unsafe fn b11_ss_f(l: &Launch, lid0: u32, x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t545: u32;
    if ((x0) < (x8)) {
        t545 = x0;
    } else {
        t545 = x8;
    }
    res = t545;
    return res;
}

#[inline(always)]
unsafe fn b11_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t546 = [vdupq_n_u32(x8); 2];
    let t547: [uint32x4_t; 2] = [vcltq_u32(x0[0], t546[0]), vcltq_u32(x0[1], t546[1])];
    let mut t548: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t549: [uint32x4_t; 2] = [vandq_u32(live[0], t547[0]), vandq_u32(live[1], t547[1])];
    let t550: [uint32x4_t; 2] = [vbicq_u32(live[0], t547[0]), vbicq_u32(live[1], t547[1])];
    {
        t548 = [vbslq_u32(t549[0], x0[0], t548[0]), vbslq_u32(t549[1], x0[1], t548[1])];
    }
    {
        let t551 = [vdupq_n_u32(x8); 2];
        t548 = [vbslq_u32(t550[0], t551[0], t548[0]), vbslq_u32(t550[1], t551[1], t548[1])];
    }
    res = t548;
    return res;
}

#[inline(always)]
unsafe fn b11_ls_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t552 = [vdupq_n_u32(x8); 2];
    let t553: [uint32x4_t; 2] = [vcltq_u32(x0[0], t552[0]), vcltq_u32(x0[1], t552[1])];
    let mut t554: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t555: [uint32x4_t; 2] = [vmvnq_u32(t553[0]), vmvnq_u32(t553[1])];
    {
        t554 = [vbslq_u32(t553[0], x0[0], t554[0]), vbslq_u32(t553[1], x0[1], t554[1])];
    }
    {
        let t556 = [vdupq_n_u32(x8); 2];
        t554 = [vbslq_u32(t555[0], t556[0], t554[0]), vbslq_u32(t555[1], t556[1], t554[1])];
    }
    res = t554;
    return res;
}

#[inline(always)]
unsafe fn b12_r0sns_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16f: u32, x16s: u32, x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t557 = b17_r0an1s_p(l, lid0, live, arg0, ((((x8) as u64)) as usize as *const u8), x16f, x16s, x24);
    x32 = t557;
    let t558: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t558 = x32;
    } else {
        let t559 = b18_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t560: u64 = t559;
        let t561 = [vdupq_n_u32((t560) as u32); 2];
        let t562 = [vdupq_n_u32(((t560) >> 32) as u32); 2];
        let t563: [uint32x4_t; 4] = [vandq_u32(x32[0], t561[0]), vandq_u32(x32[1], t561[1]), vandq_u32(x32[2], t562[0]), vandq_u32(x32[3], t562[1])];
        t558 = t563;
    }
    res = t558;
    return res;
}

#[inline(always)]
unsafe fn b12_r0sns_f(l: &Launch, lid0: u32, arg0: *const u8, x8: u32, x16f: u32, x16s: u32, x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t564 = b17_r0an1s_f(l, lid0, arg0, ((((x8) as u64)) as usize as *const u8), x16f, x16s, x24);
    x32 = t564;
    let t565: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t565 = x32;
    } else {
        let t566 = b18_s_f(l, lid0, (8u32).wrapping_mul(x24));
        let t567: u64 = t566;
        let t568 = [vdupq_n_u32((t567) as u32); 2];
        let t569 = [vdupq_n_u32(((t567) >> 32) as u32); 2];
        let t570: [uint32x4_t; 4] = [vandq_u32(x32[0], t568[0]), vandq_u32(x32[1], t568[1]), vandq_u32(x32[2], t569[0]), vandq_u32(x32[3], t569[1])];
        t565 = t570;
    }
    res = t565;
    return res;
}

#[inline(always)]
unsafe fn b12_r0lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t571: [uint32x4_t; 2] = [vaddq_u32(x8[0], x16[0]), vaddq_u32(x8[1], x16[1])];
    let t572: [uint32x4_t; 2] = [vcltq_u32(t571[0], x8[0]), vcltq_u32(t571[1], x8[1])];
    let t573: [uint32x4_t; 4] = [t571[0], t571[1], vshrq_n_u32::<31>(t572[0]), vshrq_n_u32::<31>(t572[1])];
    let t574 = b17_r0ls_p(l, lid0, live, arg0, t573, x24);
    x32 = t574;
    let t575: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t575 = x32;
    } else {
        let t576 = b18_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t577: u64 = t576;
        let t578 = [vdupq_n_u32((t577) as u32); 2];
        let t579 = [vdupq_n_u32(((t577) >> 32) as u32); 2];
        let t580: [uint32x4_t; 4] = [vandq_u32(x32[0], t578[0]), vandq_u32(x32[1], t578[1]), vandq_u32(x32[2], t579[0]), vandq_u32(x32[3], t579[1])];
        t575 = t580;
    }
    res = t575;
    return res;
}

#[inline(always)]
unsafe fn b12_r0lls_f(l: &Launch, lid0: u32, arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t581: [uint32x4_t; 2] = [vaddq_u32(x8[0], x16[0]), vaddq_u32(x8[1], x16[1])];
    let t582: [uint32x4_t; 2] = [vcltq_u32(t581[0], x8[0]), vcltq_u32(t581[1], x8[1])];
    let t583: [uint32x4_t; 4] = [t581[0], t581[1], vshrq_n_u32::<31>(t582[0]), vshrq_n_u32::<31>(t582[1])];
    let t584 = b17_r0ls_f(l, lid0, arg0, t583, x24);
    x32 = t584;
    let t585: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t585 = x32;
    } else {
        let t586 = b18_s_f(l, lid0, (8u32).wrapping_mul(x24));
        let t587: u64 = t586;
        let t588 = [vdupq_n_u32((t587) as u32); 2];
        let t589 = [vdupq_n_u32(((t587) >> 32) as u32); 2];
        let t590: [uint32x4_t; 4] = [vandq_u32(x32[0], t588[0]), vandq_u32(x32[1], t588[1]), vandq_u32(x32[2], t589[0]), vandq_u32(x32[3], t589[1])];
        t585 = t590;
    }
    res = t585;
    return res;
}

#[inline(always)]
unsafe fn b13_r0r1sss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: u32, x32: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t591 = b19_r0sss_p(l, lid0, live, arg0, x16, x24, x32);
    res = t591;
    return res;
}

#[inline(always)]
unsafe fn b13_r0r1sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2], x32: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t592 = b19_r0sls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t592;
    return res;
}

#[inline(always)]
unsafe fn b14_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
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
unsafe fn b14_sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t594 = [vdupq_n_u32(x0); 2];
    let t595: [uint32x4_t; 2] = [vcltq_u32(t594[0], x8[0]), vcltq_u32(t594[1], x8[1])];
    let mut t596: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t597: [uint32x4_t; 2] = [vandq_u32(live[0], t595[0]), vandq_u32(live[1], t595[1])];
    let t598: [uint32x4_t; 2] = [vbicq_u32(live[0], t595[0]), vbicq_u32(live[1], t595[1])];
    {
        let t599 = [vdupq_n_u32(x0); 2];
        t596 = [vbslq_u32(t597[0], t599[0], t596[0]), vbslq_u32(t597[1], t599[1], t596[1])];
    }
    {
        t596 = [vbslq_u32(t598[0], x8[0], t596[0]), vbslq_u32(t598[1], x8[1], t596[1])];
    }
    res = t596;
    return res;
}

#[inline(always)]
unsafe fn b15_ss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t600: u32;
    if ((x0) < (x8)) {
        t600 = x0;
    } else {
        t600 = x8;
    }
    res = t600;
    return res;
}

#[inline(always)]
unsafe fn b15_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t601 = [vdupq_n_u32(x8); 2];
    let t602: [uint32x4_t; 2] = [vcltq_u32(x0[0], t601[0]), vcltq_u32(x0[1], t601[1])];
    let mut t603: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t604: [uint32x4_t; 2] = [vandq_u32(live[0], t602[0]), vandq_u32(live[1], t602[1])];
    let t605: [uint32x4_t; 2] = [vbicq_u32(live[0], t602[0]), vbicq_u32(live[1], t602[1])];
    {
        t603 = [vbslq_u32(t604[0], x0[0], t603[0]), vbslq_u32(t604[1], x0[1], t603[1])];
    }
    {
        let t606 = [vdupq_n_u32(x8); 2];
        t603 = [vbslq_u32(t605[0], t606[0], t603[0]), vbslq_u32(t605[1], t606[1], t603[1])];
    }
    res = t603;
    return res;
}

#[inline(always)]
unsafe fn b15_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t607: [uint32x4_t; 2] = [vcltq_u32(x0[0], x8[0]), vcltq_u32(x0[1], x8[1])];
    let mut t608: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t609: [uint32x4_t; 2] = [vandq_u32(live[0], t607[0]), vandq_u32(live[1], t607[1])];
    let t610: [uint32x4_t; 2] = [vbicq_u32(live[0], t607[0]), vbicq_u32(live[1], t607[1])];
    {
        t608 = [vbslq_u32(t609[0], x0[0], t608[0]), vbslq_u32(t609[1], x0[1], t608[1])];
    }
    {
        t608 = [vbslq_u32(t610[0], x8[0], t608[0]), vbslq_u32(t610[1], x8[1], t608[1])];
    }
    res = t608;
    return res;
}

#[inline(always)]
unsafe fn b16_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t611: [uint32x4_t; 2] = [vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[0])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[2]))))), vaddq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[1])))), vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u32(x0[3])))))];
    res = t611;
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
    let mut t612: bool = ((x32) != (0u32));
    if t612 {
        t612 = (((x32).wrapping_add((x16).wrapping_mul(8u32))) > (64u32));
    }
    let t613: u64;
    if t612 {
        t613 = ((x40) | (((x24.wrapping_add((1u64).wrapping_mul(8u64) as usize) as *const u64).read_unaligned()).wrapping_shl(((((64u32).wrapping_sub(x32)) as u64)) as u32)));
    } else {
        t613 = x40;
    }
    res = t613;
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
        let t614: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
        let t615: [uint32x4_t; 4] = [vandq_u32(x8[0], t614[0]), vandq_u32(x8[1], t614[1]), vandq_u32(x8[2], t614[2]), vandq_u32(x8[3], t614[3])];
        let t616: u64 = ((arg0) as usize as u64);
        let t617 = [vdupq_n_u32((t616) as u32); 2];
        let t618 = [vdupq_n_u32(((t616) >> 32) as u32); 2];
        let t619: [uint32x4_t; 2] = [vaddq_u32(t617[0], t615[0]), vaddq_u32(t617[1], t615[1])];
        let t620: [uint32x4_t; 2] = [vcltq_u32(t619[0], t617[0]), vcltq_u32(t619[1], t617[1])];
        let t621: [uint32x4_t; 4] = [t619[0], t619[1], vsubq_u32(vaddq_u32(t618[0], t615[2]), t620[0]), vsubq_u32(vaddq_u32(t618[1], t615[3]), t620[1])];
        let mut t622: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t621[2]) as u64) << 32 | vgetq_lane_u32::<0>(t621[0]) as u64) as usize as *const u64).read_unaligned(); t622[0] = vsetq_lane_u32::<0>(v as u32, t622[0]); t622[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t622[2]); }
        { let v = (((vgetq_lane_u32::<1>(t621[2]) as u64) << 32 | vgetq_lane_u32::<1>(t621[0]) as u64) as usize as *const u64).read_unaligned(); t622[0] = vsetq_lane_u32::<1>(v as u32, t622[0]); t622[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t622[2]); }
        { let v = (((vgetq_lane_u32::<2>(t621[2]) as u64) << 32 | vgetq_lane_u32::<2>(t621[0]) as u64) as usize as *const u64).read_unaligned(); t622[0] = vsetq_lane_u32::<2>(v as u32, t622[0]); t622[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t622[2]); }
        { let v = (((vgetq_lane_u32::<3>(t621[2]) as u64) << 32 | vgetq_lane_u32::<3>(t621[0]) as u64) as usize as *const u64).read_unaligned(); t622[0] = vsetq_lane_u32::<3>(v as u32, t622[0]); t622[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t622[2]); }
        { let v = (((vgetq_lane_u32::<0>(t621[3]) as u64) << 32 | vgetq_lane_u32::<0>(t621[1]) as u64) as usize as *const u64).read_unaligned(); t622[1] = vsetq_lane_u32::<0>(v as u32, t622[1]); t622[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t622[3]); }
        { let v = (((vgetq_lane_u32::<1>(t621[3]) as u64) << 32 | vgetq_lane_u32::<1>(t621[1]) as u64) as usize as *const u64).read_unaligned(); t622[1] = vsetq_lane_u32::<1>(v as u32, t622[1]); t622[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t622[3]); }
        { let v = (((vgetq_lane_u32::<2>(t621[3]) as u64) << 32 | vgetq_lane_u32::<2>(t621[1]) as u64) as usize as *const u64).read_unaligned(); t622[1] = vsetq_lane_u32::<2>(v as u32, t622[1]); t622[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t622[3]); }
        { let v = (((vgetq_lane_u32::<3>(t621[3]) as u64) << 32 | vgetq_lane_u32::<3>(t621[1]) as u64) as usize as *const u64).read_unaligned(); t622[1] = vsetq_lane_u32::<3>(v as u32, t622[1]); t622[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t622[3]); }
        res = t622;
        return res;
    }
    let t623: u64 = !(7u64);
    let t624 = [vdupq_n_u32((t623) as u32); 2];
    let t625 = [vdupq_n_u32(((t623) >> 32) as u32); 2];
    let t626: [uint32x4_t; 4] = [vandq_u32(x8[0], t624[0]), vandq_u32(x8[1], t624[1]), vandq_u32(x8[2], t625[0]), vandq_u32(x8[3], t625[1])];
    let t627: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t628: [uint32x4_t; 4] = [vandq_u32(t626[0], t627[0]), vandq_u32(t626[1], t627[1]), vandq_u32(t626[2], t627[2]), vandq_u32(t626[3], t627[3])];
    let t629: u64 = ((arg0) as usize as u64);
    let t630 = [vdupq_n_u32((t629) as u32); 2];
    let t631 = [vdupq_n_u32(((t629) >> 32) as u32); 2];
    let t632: [uint32x4_t; 2] = [vaddq_u32(t630[0], t628[0]), vaddq_u32(t630[1], t628[1])];
    let t633: [uint32x4_t; 2] = [vcltq_u32(t632[0], t630[0]), vcltq_u32(t632[1], t630[1])];
    let t634: [uint32x4_t; 4] = [t632[0], t632[1], vsubq_u32(vaddq_u32(t631[0], t628[2]), t633[0]), vsubq_u32(vaddq_u32(t631[1], t628[3]), t633[1])];
    x24 = t634;
    let t635: u64 = 7u64;
    let t636 = [vdupq_n_u32((t635) as u32); 2];
    let t637: [uint32x4_t; 2] = [vandq_u32(x8[0], t636[0]), vandq_u32(x8[1], t636[1])];
    let t638 = [vdupq_n_u32(8u32); 2];
    let t639: [uint32x4_t; 2] = [vmulq_u32(t637[0], t638[0]), vmulq_u32(t637[1], t638[1])];
    x32 = t639;
    let mut t640: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t640[0] = vsetq_lane_u32::<0>(v as u32, t640[0]); t640[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t640[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t640[0] = vsetq_lane_u32::<1>(v as u32, t640[0]); t640[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t640[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t640[0] = vsetq_lane_u32::<2>(v as u32, t640[0]); t640[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t640[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t640[0] = vsetq_lane_u32::<3>(v as u32, t640[0]); t640[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t640[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t640[1] = vsetq_lane_u32::<0>(v as u32, t640[1]); t640[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t640[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t640[1] = vsetq_lane_u32::<1>(v as u32, t640[1]); t640[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t640[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t640[1] = vsetq_lane_u32::<2>(v as u32, t640[1]); t640[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t640[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t640[1] = vsetq_lane_u32::<3>(v as u32, t640[1]); t640[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t640[3]); }
    let t641: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t642: [uint32x4_t; 2] = [vandq_u32(t641[0], vdupq_n_u32(63)), vandq_u32(t641[1], vdupq_n_u32(63))];
    let t643: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t640[0], vnegq_s32(vreinterpretq_s32_u32(t642[0]))), vshlq_u32(t640[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t642[0])))), vorrq_u32(vshlq_u32(t640[1], vnegq_s32(vreinterpretq_s32_u32(t642[1]))), vshlq_u32(t640[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t642[1])))), vshlq_u32(t640[2], vnegq_s32(vreinterpretq_s32_u32(t642[0]))), vshlq_u32(t640[3], vnegq_s32(vreinterpretq_s32_u32(t642[1])))];
    x40 = t643;
    let t644 = [vdupq_n_u32(0u32); 2];
    let t645: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t644[0])), vmvnq_u32(vceqq_u32(x32[1], t644[1]))];
    let t646 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t647: [uint32x4_t; 2] = [vaddq_u32(x32[0], t646[0]), vaddq_u32(x32[1], t646[1])];
    let t648 = [vdupq_n_u32(64u32); 2];
    let t649: [uint32x4_t; 2] = [vcgtq_u32(t647[0], t648[0]), vcgtq_u32(t647[1], t648[1])];
    let t650: [uint32x4_t; 2] = [vandq_u32(t645[0], t649[0]), vandq_u32(t645[1], t649[1])];
    let mut t651: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t652: [uint32x4_t; 2] = [vandq_u32(live[0], t650[0]), vandq_u32(live[1], t650[1])];
    let t653: [uint32x4_t; 2] = [vbicq_u32(live[0], t650[0]), vbicq_u32(live[1], t650[1])];
    if any(&t652) {
        let t654: u64 = (1u64).wrapping_mul(8u64);
        let t655 = [vdupq_n_u32((t654) as u32); 2];
        let t656 = [vdupq_n_u32(((t654) >> 32) as u32); 2];
        let t657: [uint32x4_t; 2] = [vaddq_u32(x24[0], t655[0]), vaddq_u32(x24[1], t655[1])];
        let t658: [uint32x4_t; 2] = [vcltq_u32(t657[0], x24[0]), vcltq_u32(t657[1], x24[1])];
        let t659: [uint32x4_t; 4] = [t657[0], t657[1], vsubq_u32(vaddq_u32(x24[2], t656[0]), t658[0]), vsubq_u32(vaddq_u32(x24[3], t656[1]), t658[1])];
        let mut t660: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t659[2]) as u64) << 32 | vgetq_lane_u32::<0>(t659[0]) as u64) as usize as *const u64).read_unaligned(); t660[0] = vsetq_lane_u32::<0>(v as u32, t660[0]); t660[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t660[2]); }
        { let v = (((vgetq_lane_u32::<1>(t659[2]) as u64) << 32 | vgetq_lane_u32::<1>(t659[0]) as u64) as usize as *const u64).read_unaligned(); t660[0] = vsetq_lane_u32::<1>(v as u32, t660[0]); t660[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t660[2]); }
        { let v = (((vgetq_lane_u32::<2>(t659[2]) as u64) << 32 | vgetq_lane_u32::<2>(t659[0]) as u64) as usize as *const u64).read_unaligned(); t660[0] = vsetq_lane_u32::<2>(v as u32, t660[0]); t660[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t660[2]); }
        { let v = (((vgetq_lane_u32::<3>(t659[2]) as u64) << 32 | vgetq_lane_u32::<3>(t659[0]) as u64) as usize as *const u64).read_unaligned(); t660[0] = vsetq_lane_u32::<3>(v as u32, t660[0]); t660[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t660[2]); }
        { let v = (((vgetq_lane_u32::<0>(t659[3]) as u64) << 32 | vgetq_lane_u32::<0>(t659[1]) as u64) as usize as *const u64).read_unaligned(); t660[1] = vsetq_lane_u32::<0>(v as u32, t660[1]); t660[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t660[3]); }
        { let v = (((vgetq_lane_u32::<1>(t659[3]) as u64) << 32 | vgetq_lane_u32::<1>(t659[1]) as u64) as usize as *const u64).read_unaligned(); t660[1] = vsetq_lane_u32::<1>(v as u32, t660[1]); t660[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t660[3]); }
        { let v = (((vgetq_lane_u32::<2>(t659[3]) as u64) << 32 | vgetq_lane_u32::<2>(t659[1]) as u64) as usize as *const u64).read_unaligned(); t660[1] = vsetq_lane_u32::<2>(v as u32, t660[1]); t660[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t660[3]); }
        { let v = (((vgetq_lane_u32::<3>(t659[3]) as u64) << 32 | vgetq_lane_u32::<3>(t659[1]) as u64) as usize as *const u64).read_unaligned(); t660[1] = vsetq_lane_u32::<3>(v as u32, t660[1]); t660[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t660[3]); }
        let t661: [uint32x4_t; 2] = [vsubq_u32(t648[0], x32[0]), vsubq_u32(t648[1], x32[1])];
        let t662: [uint32x4_t; 2] = [t661[0], t661[1]];
        let t663: [uint32x4_t; 2] = [vandq_u32(t662[0], vdupq_n_u32(63)), vandq_u32(t662[1], vdupq_n_u32(63))];
        let t664: [uint32x4_t; 4] = [vshlq_u32(t660[0], vreinterpretq_s32_u32(t663[0])), vshlq_u32(t660[1], vreinterpretq_s32_u32(t663[1])), vorrq_u32(vshlq_u32(t660[2], vreinterpretq_s32_u32(t663[0])), vshlq_u32(t660[0], vsubq_s32(vreinterpretq_s32_u32(t663[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t660[3], vreinterpretq_s32_u32(t663[1])), vshlq_u32(t660[1], vsubq_s32(vreinterpretq_s32_u32(t663[1]), vdupq_n_s32(32))))];
        let t665: [uint32x4_t; 4] = [vorrq_u32(x40[0], t664[0]), vorrq_u32(x40[1], t664[1]), vorrq_u32(x40[2], t664[2]), vorrq_u32(x40[3], t664[3])];
        t651 = [vbslq_u32(t652[0], t665[0], t651[0]), vbslq_u32(t652[1], t665[1], t651[1]), vbslq_u32(t652[0], t665[2], t651[2]), vbslq_u32(t652[1], t665[3], t651[3])];
    }
    {
        t651 = [vbslq_u32(t653[0], x40[0], t651[0]), vbslq_u32(t653[1], x40[1], t651[1]), vbslq_u32(t653[0], x40[2], t651[2]), vbslq_u32(t653[1], x40[3], t651[3])];
    }
    res = t651;
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
        let t666: u64 = ((arg0) as usize as u64);
        let t667 = [vdupq_n_u32((t666) as u32); 2];
        let t668 = [vdupq_n_u32(((t666) >> 32) as u32); 2];
        let t669: [uint32x4_t; 2] = [vaddq_u32(t667[0], x8[0]), vaddq_u32(t667[1], x8[1])];
        let t670: [uint32x4_t; 2] = [vcltq_u32(t669[0], t667[0]), vcltq_u32(t669[1], t667[1])];
        let t671: [uint32x4_t; 4] = [t669[0], t669[1], vsubq_u32(vaddq_u32(t668[0], x8[2]), t670[0]), vsubq_u32(vaddq_u32(t668[1], x8[3]), t670[1])];
        let mut t672: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t671[2]) as u64) << 32 | vgetq_lane_u32::<0>(t671[0]) as u64) as usize as *const u64).read_unaligned(); t672[0] = vsetq_lane_u32::<0>(v as u32, t672[0]); t672[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t672[2]); }
        { let v = (((vgetq_lane_u32::<1>(t671[2]) as u64) << 32 | vgetq_lane_u32::<1>(t671[0]) as u64) as usize as *const u64).read_unaligned(); t672[0] = vsetq_lane_u32::<1>(v as u32, t672[0]); t672[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t672[2]); }
        { let v = (((vgetq_lane_u32::<2>(t671[2]) as u64) << 32 | vgetq_lane_u32::<2>(t671[0]) as u64) as usize as *const u64).read_unaligned(); t672[0] = vsetq_lane_u32::<2>(v as u32, t672[0]); t672[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t672[2]); }
        { let v = (((vgetq_lane_u32::<3>(t671[2]) as u64) << 32 | vgetq_lane_u32::<3>(t671[0]) as u64) as usize as *const u64).read_unaligned(); t672[0] = vsetq_lane_u32::<3>(v as u32, t672[0]); t672[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t672[2]); }
        { let v = (((vgetq_lane_u32::<0>(t671[3]) as u64) << 32 | vgetq_lane_u32::<0>(t671[1]) as u64) as usize as *const u64).read_unaligned(); t672[1] = vsetq_lane_u32::<0>(v as u32, t672[1]); t672[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t672[3]); }
        { let v = (((vgetq_lane_u32::<1>(t671[3]) as u64) << 32 | vgetq_lane_u32::<1>(t671[1]) as u64) as usize as *const u64).read_unaligned(); t672[1] = vsetq_lane_u32::<1>(v as u32, t672[1]); t672[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t672[3]); }
        { let v = (((vgetq_lane_u32::<2>(t671[3]) as u64) << 32 | vgetq_lane_u32::<2>(t671[1]) as u64) as usize as *const u64).read_unaligned(); t672[1] = vsetq_lane_u32::<2>(v as u32, t672[1]); t672[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t672[3]); }
        { let v = (((vgetq_lane_u32::<3>(t671[3]) as u64) << 32 | vgetq_lane_u32::<3>(t671[1]) as u64) as usize as *const u64).read_unaligned(); t672[1] = vsetq_lane_u32::<3>(v as u32, t672[1]); t672[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t672[3]); }
        res = t672;
        return res;
    }
    let t673: u64 = !(7u64);
    let t674 = [vdupq_n_u32((t673) as u32); 2];
    let t675 = [vdupq_n_u32(((t673) >> 32) as u32); 2];
    let t676: [uint32x4_t; 4] = [vandq_u32(x8[0], t674[0]), vandq_u32(x8[1], t674[1]), vandq_u32(x8[2], t675[0]), vandq_u32(x8[3], t675[1])];
    let t677: u64 = ((arg0) as usize as u64);
    let t678 = [vdupq_n_u32((t677) as u32); 2];
    let t679 = [vdupq_n_u32(((t677) >> 32) as u32); 2];
    let t680: [uint32x4_t; 2] = [vaddq_u32(t678[0], t676[0]), vaddq_u32(t678[1], t676[1])];
    let t681: [uint32x4_t; 2] = [vcltq_u32(t680[0], t678[0]), vcltq_u32(t680[1], t678[1])];
    let t682: [uint32x4_t; 4] = [t680[0], t680[1], vsubq_u32(vaddq_u32(t679[0], t676[2]), t681[0]), vsubq_u32(vaddq_u32(t679[1], t676[3]), t681[1])];
    x24 = t682;
    let t683: u64 = 7u64;
    let t684 = [vdupq_n_u32((t683) as u32); 2];
    let t685: [uint32x4_t; 2] = [vandq_u32(x8[0], t684[0]), vandq_u32(x8[1], t684[1])];
    let t686 = [vdupq_n_u32(8u32); 2];
    let t687: [uint32x4_t; 2] = [vmulq_u32(t685[0], t686[0]), vmulq_u32(t685[1], t686[1])];
    x32 = t687;
    let mut t688: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t688[0] = vsetq_lane_u32::<0>(v as u32, t688[0]); t688[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t688[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t688[0] = vsetq_lane_u32::<1>(v as u32, t688[0]); t688[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t688[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t688[0] = vsetq_lane_u32::<2>(v as u32, t688[0]); t688[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t688[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t688[0] = vsetq_lane_u32::<3>(v as u32, t688[0]); t688[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t688[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t688[1] = vsetq_lane_u32::<0>(v as u32, t688[1]); t688[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t688[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t688[1] = vsetq_lane_u32::<1>(v as u32, t688[1]); t688[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t688[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t688[1] = vsetq_lane_u32::<2>(v as u32, t688[1]); t688[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t688[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t688[1] = vsetq_lane_u32::<3>(v as u32, t688[1]); t688[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t688[3]); }
    let t689: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t690: [uint32x4_t; 2] = [vandq_u32(t689[0], vdupq_n_u32(63)), vandq_u32(t689[1], vdupq_n_u32(63))];
    let t691: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t688[0], vnegq_s32(vreinterpretq_s32_u32(t690[0]))), vshlq_u32(t688[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t690[0])))), vorrq_u32(vshlq_u32(t688[1], vnegq_s32(vreinterpretq_s32_u32(t690[1]))), vshlq_u32(t688[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t690[1])))), vshlq_u32(t688[2], vnegq_s32(vreinterpretq_s32_u32(t690[0]))), vshlq_u32(t688[3], vnegq_s32(vreinterpretq_s32_u32(t690[1])))];
    x40 = t691;
    let t692 = [vdupq_n_u32(0u32); 2];
    let t693: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t692[0])), vmvnq_u32(vceqq_u32(x32[1], t692[1]))];
    let t694 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t695: [uint32x4_t; 2] = [vaddq_u32(x32[0], t694[0]), vaddq_u32(x32[1], t694[1])];
    let t696 = [vdupq_n_u32(64u32); 2];
    let t697: [uint32x4_t; 2] = [vcgtq_u32(t695[0], t696[0]), vcgtq_u32(t695[1], t696[1])];
    let t698: [uint32x4_t; 2] = [vandq_u32(t693[0], t697[0]), vandq_u32(t693[1], t697[1])];
    let mut t699: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t700: [uint32x4_t; 2] = [vmvnq_u32(t698[0]), vmvnq_u32(t698[1])];
    if any(&t698) {
        let t701: u64 = (1u64).wrapping_mul(8u64);
        let t702 = [vdupq_n_u32((t701) as u32); 2];
        let t703 = [vdupq_n_u32(((t701) >> 32) as u32); 2];
        let t704: [uint32x4_t; 2] = [vaddq_u32(x24[0], t702[0]), vaddq_u32(x24[1], t702[1])];
        let t705: [uint32x4_t; 2] = [vcltq_u32(t704[0], x24[0]), vcltq_u32(t704[1], x24[1])];
        let t706: [uint32x4_t; 4] = [t704[0], t704[1], vsubq_u32(vaddq_u32(x24[2], t703[0]), t705[0]), vsubq_u32(vaddq_u32(x24[3], t703[1]), t705[1])];
        let mut t707: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t706[2]) as u64) << 32 | vgetq_lane_u32::<0>(t706[0]) as u64) as usize as *const u64).read_unaligned(); t707[0] = vsetq_lane_u32::<0>(v as u32, t707[0]); t707[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t707[2]); }
        { let v = (((vgetq_lane_u32::<1>(t706[2]) as u64) << 32 | vgetq_lane_u32::<1>(t706[0]) as u64) as usize as *const u64).read_unaligned(); t707[0] = vsetq_lane_u32::<1>(v as u32, t707[0]); t707[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t707[2]); }
        { let v = (((vgetq_lane_u32::<2>(t706[2]) as u64) << 32 | vgetq_lane_u32::<2>(t706[0]) as u64) as usize as *const u64).read_unaligned(); t707[0] = vsetq_lane_u32::<2>(v as u32, t707[0]); t707[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t707[2]); }
        { let v = (((vgetq_lane_u32::<3>(t706[2]) as u64) << 32 | vgetq_lane_u32::<3>(t706[0]) as u64) as usize as *const u64).read_unaligned(); t707[0] = vsetq_lane_u32::<3>(v as u32, t707[0]); t707[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t707[2]); }
        { let v = (((vgetq_lane_u32::<0>(t706[3]) as u64) << 32 | vgetq_lane_u32::<0>(t706[1]) as u64) as usize as *const u64).read_unaligned(); t707[1] = vsetq_lane_u32::<0>(v as u32, t707[1]); t707[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t707[3]); }
        { let v = (((vgetq_lane_u32::<1>(t706[3]) as u64) << 32 | vgetq_lane_u32::<1>(t706[1]) as u64) as usize as *const u64).read_unaligned(); t707[1] = vsetq_lane_u32::<1>(v as u32, t707[1]); t707[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t707[3]); }
        { let v = (((vgetq_lane_u32::<2>(t706[3]) as u64) << 32 | vgetq_lane_u32::<2>(t706[1]) as u64) as usize as *const u64).read_unaligned(); t707[1] = vsetq_lane_u32::<2>(v as u32, t707[1]); t707[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t707[3]); }
        { let v = (((vgetq_lane_u32::<3>(t706[3]) as u64) << 32 | vgetq_lane_u32::<3>(t706[1]) as u64) as usize as *const u64).read_unaligned(); t707[1] = vsetq_lane_u32::<3>(v as u32, t707[1]); t707[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t707[3]); }
        let t708: [uint32x4_t; 2] = [vsubq_u32(t696[0], x32[0]), vsubq_u32(t696[1], x32[1])];
        let t709: [uint32x4_t; 2] = [t708[0], t708[1]];
        let t710: [uint32x4_t; 2] = [vandq_u32(t709[0], vdupq_n_u32(63)), vandq_u32(t709[1], vdupq_n_u32(63))];
        let t711: [uint32x4_t; 4] = [vshlq_u32(t707[0], vreinterpretq_s32_u32(t710[0])), vshlq_u32(t707[1], vreinterpretq_s32_u32(t710[1])), vorrq_u32(vshlq_u32(t707[2], vreinterpretq_s32_u32(t710[0])), vshlq_u32(t707[0], vsubq_s32(vreinterpretq_s32_u32(t710[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t707[3], vreinterpretq_s32_u32(t710[1])), vshlq_u32(t707[1], vsubq_s32(vreinterpretq_s32_u32(t710[1]), vdupq_n_s32(32))))];
        let t712: [uint32x4_t; 4] = [vorrq_u32(x40[0], t711[0]), vorrq_u32(x40[1], t711[1]), vorrq_u32(x40[2], t711[2]), vorrq_u32(x40[3], t711[3])];
        t699 = [vbslq_u32(t698[0], t712[0], t699[0]), vbslq_u32(t698[1], t712[1], t699[1]), vbslq_u32(t698[0], t712[2], t699[2]), vbslq_u32(t698[1], t712[3], t699[3])];
    }
    {
        t699 = [vbslq_u32(t700[0], x40[0], t699[0]), vbslq_u32(t700[1], x40[1], t699[1]), vbslq_u32(t700[0], x40[2], t699[2]), vbslq_u32(t700[1], x40[3], t699[3])];
    }
    res = t699;
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
        let t713 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x8i[0]), vandq_u32(live[1], x8i[1]))));
        let t714: [uint32x4_t; 2] = [vbslq_u32(live[0], x8i[0], t713), vbslq_u32(live[1], x8i[1], t713)];
        let mut t715: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t714[0]) as usize * 1) as *const u64).read_unaligned(); t715[0] = vsetq_lane_u32::<0>(v as u32, t715[0]); t715[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t715[2]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t714[0]) as usize * 1) as *const u64).read_unaligned(); t715[0] = vsetq_lane_u32::<1>(v as u32, t715[0]); t715[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t715[2]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t714[0]) as usize * 1) as *const u64).read_unaligned(); t715[0] = vsetq_lane_u32::<2>(v as u32, t715[0]); t715[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t715[2]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t714[0]) as usize * 1) as *const u64).read_unaligned(); t715[0] = vsetq_lane_u32::<3>(v as u32, t715[0]); t715[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t715[2]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t714[1]) as usize * 1) as *const u64).read_unaligned(); t715[1] = vsetq_lane_u32::<0>(v as u32, t715[1]); t715[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t715[3]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t714[1]) as usize * 1) as *const u64).read_unaligned(); t715[1] = vsetq_lane_u32::<1>(v as u32, t715[1]); t715[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t715[3]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t714[1]) as usize * 1) as *const u64).read_unaligned(); t715[1] = vsetq_lane_u32::<2>(v as u32, t715[1]); t715[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t715[3]); }
        { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t714[1]) as usize * 1) as *const u64).read_unaligned(); t715[1] = vsetq_lane_u32::<3>(v as u32, t715[1]); t715[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t715[3]); }
        res = t715;
        return res;
    }
    let t716: [uint32x4_t; 2] = [x8i[0], x8i[1]];
    let t717: u64 = ((x8b) as usize as u64);
    let t718 = [vdupq_n_u32((t717) as u32); 2];
    let t719 = [vdupq_n_u32(((t717) >> 32) as u32); 2];
    let t720: [uint32x4_t; 2] = [vaddq_u32(t718[0], t716[0]), vaddq_u32(t718[1], t716[1])];
    let t721: [uint32x4_t; 2] = [vcltq_u32(t720[0], t718[0]), vcltq_u32(t720[1], t718[1])];
    let t722: [uint32x4_t; 4] = [t720[0], t720[1], vsubq_u32(vaddq_u32(t719[0], vdupq_n_u32(0)), t721[0]), vsubq_u32(vaddq_u32(t719[1], vdupq_n_u32(0)), t721[1])];
    let t723: u64 = !(7u64);
    let t724 = [vdupq_n_u32((t723) as u32); 2];
    let t725 = [vdupq_n_u32(((t723) >> 32) as u32); 2];
    let t726: [uint32x4_t; 4] = [vandq_u32(t722[0], t724[0]), vandq_u32(t722[1], t724[1]), vandq_u32(t722[2], t725[0]), vandq_u32(t722[3], t725[1])];
    let t727: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t728: [uint32x4_t; 4] = [vandq_u32(t726[0], t727[0]), vandq_u32(t726[1], t727[1]), vandq_u32(t726[2], t727[2]), vandq_u32(t726[3], t727[3])];
    let t729: u64 = ((arg0) as usize as u64);
    let t730 = [vdupq_n_u32((t729) as u32); 2];
    let t731 = [vdupq_n_u32(((t729) >> 32) as u32); 2];
    let t732: [uint32x4_t; 2] = [vaddq_u32(t730[0], t728[0]), vaddq_u32(t730[1], t728[1])];
    let t733: [uint32x4_t; 2] = [vcltq_u32(t732[0], t730[0]), vcltq_u32(t732[1], t730[1])];
    let t734: [uint32x4_t; 4] = [t732[0], t732[1], vsubq_u32(vaddq_u32(t731[0], t728[2]), t733[0]), vsubq_u32(vaddq_u32(t731[1], t728[3]), t733[1])];
    x24 = t734;
    let t735: [uint32x4_t; 2] = [x8i[0], x8i[1]];
    let t736: u64 = ((x8b) as usize as u64);
    let t737 = [vdupq_n_u32((t736) as u32); 2];
    let t738 = [vdupq_n_u32(((t736) >> 32) as u32); 2];
    let t739: [uint32x4_t; 2] = [vaddq_u32(t737[0], t735[0]), vaddq_u32(t737[1], t735[1])];
    let t740: [uint32x4_t; 2] = [vcltq_u32(t739[0], t737[0]), vcltq_u32(t739[1], t737[1])];
    let t741: [uint32x4_t; 4] = [t739[0], t739[1], vsubq_u32(vaddq_u32(t738[0], vdupq_n_u32(0)), t740[0]), vsubq_u32(vaddq_u32(t738[1], vdupq_n_u32(0)), t740[1])];
    let t742: u64 = 7u64;
    let t743 = [vdupq_n_u32((t742) as u32); 2];
    let t744: [uint32x4_t; 2] = [vandq_u32(t741[0], t743[0]), vandq_u32(t741[1], t743[1])];
    let t745 = [vdupq_n_u32(8u32); 2];
    let t746: [uint32x4_t; 2] = [vmulq_u32(t744[0], t745[0]), vmulq_u32(t744[1], t745[1])];
    x32 = t746;
    let mut t747: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t747[0] = vsetq_lane_u32::<0>(v as u32, t747[0]); t747[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t747[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t747[0] = vsetq_lane_u32::<1>(v as u32, t747[0]); t747[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t747[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t747[0] = vsetq_lane_u32::<2>(v as u32, t747[0]); t747[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t747[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t747[0] = vsetq_lane_u32::<3>(v as u32, t747[0]); t747[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t747[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t747[1] = vsetq_lane_u32::<0>(v as u32, t747[1]); t747[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t747[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t747[1] = vsetq_lane_u32::<1>(v as u32, t747[1]); t747[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t747[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t747[1] = vsetq_lane_u32::<2>(v as u32, t747[1]); t747[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t747[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t747[1] = vsetq_lane_u32::<3>(v as u32, t747[1]); t747[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t747[3]); }
    let t748: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t749: [uint32x4_t; 2] = [vandq_u32(t748[0], vdupq_n_u32(63)), vandq_u32(t748[1], vdupq_n_u32(63))];
    let t750: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t747[0], vnegq_s32(vreinterpretq_s32_u32(t749[0]))), vshlq_u32(t747[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t749[0])))), vorrq_u32(vshlq_u32(t747[1], vnegq_s32(vreinterpretq_s32_u32(t749[1]))), vshlq_u32(t747[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t749[1])))), vshlq_u32(t747[2], vnegq_s32(vreinterpretq_s32_u32(t749[0]))), vshlq_u32(t747[3], vnegq_s32(vreinterpretq_s32_u32(t749[1])))];
    x40 = t750;
    let t751 = [vdupq_n_u32(0u32); 2];
    let t752: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t751[0])), vmvnq_u32(vceqq_u32(x32[1], t751[1]))];
    let t753 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t754: [uint32x4_t; 2] = [vaddq_u32(x32[0], t753[0]), vaddq_u32(x32[1], t753[1])];
    let t755 = [vdupq_n_u32(64u32); 2];
    let t756: [uint32x4_t; 2] = [vcgtq_u32(t754[0], t755[0]), vcgtq_u32(t754[1], t755[1])];
    let t757: [uint32x4_t; 2] = [vandq_u32(t752[0], t756[0]), vandq_u32(t752[1], t756[1])];
    let mut t758: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t759: [uint32x4_t; 2] = [vandq_u32(live[0], t757[0]), vandq_u32(live[1], t757[1])];
    let t760: [uint32x4_t; 2] = [vbicq_u32(live[0], t757[0]), vbicq_u32(live[1], t757[1])];
    if any(&t759) {
        let t761: u64 = (1u64).wrapping_mul(8u64);
        let t762 = [vdupq_n_u32((t761) as u32); 2];
        let t763 = [vdupq_n_u32(((t761) >> 32) as u32); 2];
        let t764: [uint32x4_t; 2] = [vaddq_u32(x24[0], t762[0]), vaddq_u32(x24[1], t762[1])];
        let t765: [uint32x4_t; 2] = [vcltq_u32(t764[0], x24[0]), vcltq_u32(t764[1], x24[1])];
        let t766: [uint32x4_t; 4] = [t764[0], t764[1], vsubq_u32(vaddq_u32(x24[2], t763[0]), t765[0]), vsubq_u32(vaddq_u32(x24[3], t763[1]), t765[1])];
        let mut t767: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t766[2]) as u64) << 32 | vgetq_lane_u32::<0>(t766[0]) as u64) as usize as *const u64).read_unaligned(); t767[0] = vsetq_lane_u32::<0>(v as u32, t767[0]); t767[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t767[2]); }
        { let v = (((vgetq_lane_u32::<1>(t766[2]) as u64) << 32 | vgetq_lane_u32::<1>(t766[0]) as u64) as usize as *const u64).read_unaligned(); t767[0] = vsetq_lane_u32::<1>(v as u32, t767[0]); t767[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t767[2]); }
        { let v = (((vgetq_lane_u32::<2>(t766[2]) as u64) << 32 | vgetq_lane_u32::<2>(t766[0]) as u64) as usize as *const u64).read_unaligned(); t767[0] = vsetq_lane_u32::<2>(v as u32, t767[0]); t767[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t767[2]); }
        { let v = (((vgetq_lane_u32::<3>(t766[2]) as u64) << 32 | vgetq_lane_u32::<3>(t766[0]) as u64) as usize as *const u64).read_unaligned(); t767[0] = vsetq_lane_u32::<3>(v as u32, t767[0]); t767[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t767[2]); }
        { let v = (((vgetq_lane_u32::<0>(t766[3]) as u64) << 32 | vgetq_lane_u32::<0>(t766[1]) as u64) as usize as *const u64).read_unaligned(); t767[1] = vsetq_lane_u32::<0>(v as u32, t767[1]); t767[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t767[3]); }
        { let v = (((vgetq_lane_u32::<1>(t766[3]) as u64) << 32 | vgetq_lane_u32::<1>(t766[1]) as u64) as usize as *const u64).read_unaligned(); t767[1] = vsetq_lane_u32::<1>(v as u32, t767[1]); t767[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t767[3]); }
        { let v = (((vgetq_lane_u32::<2>(t766[3]) as u64) << 32 | vgetq_lane_u32::<2>(t766[1]) as u64) as usize as *const u64).read_unaligned(); t767[1] = vsetq_lane_u32::<2>(v as u32, t767[1]); t767[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t767[3]); }
        { let v = (((vgetq_lane_u32::<3>(t766[3]) as u64) << 32 | vgetq_lane_u32::<3>(t766[1]) as u64) as usize as *const u64).read_unaligned(); t767[1] = vsetq_lane_u32::<3>(v as u32, t767[1]); t767[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t767[3]); }
        let t768: [uint32x4_t; 2] = [vsubq_u32(t755[0], x32[0]), vsubq_u32(t755[1], x32[1])];
        let t769: [uint32x4_t; 2] = [t768[0], t768[1]];
        let t770: [uint32x4_t; 2] = [vandq_u32(t769[0], vdupq_n_u32(63)), vandq_u32(t769[1], vdupq_n_u32(63))];
        let t771: [uint32x4_t; 4] = [vshlq_u32(t767[0], vreinterpretq_s32_u32(t770[0])), vshlq_u32(t767[1], vreinterpretq_s32_u32(t770[1])), vorrq_u32(vshlq_u32(t767[2], vreinterpretq_s32_u32(t770[0])), vshlq_u32(t767[0], vsubq_s32(vreinterpretq_s32_u32(t770[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t767[3], vreinterpretq_s32_u32(t770[1])), vshlq_u32(t767[1], vsubq_s32(vreinterpretq_s32_u32(t770[1]), vdupq_n_s32(32))))];
        let t772: [uint32x4_t; 4] = [vorrq_u32(x40[0], t771[0]), vorrq_u32(x40[1], t771[1]), vorrq_u32(x40[2], t771[2]), vorrq_u32(x40[3], t771[3])];
        t758 = [vbslq_u32(t759[0], t772[0], t758[0]), vbslq_u32(t759[1], t772[1], t758[1]), vbslq_u32(t759[0], t772[2], t758[2]), vbslq_u32(t759[1], t772[3], t758[3])];
    }
    {
        t758 = [vbslq_u32(t760[0], x40[0], t758[0]), vbslq_u32(t760[1], x40[1], t758[1]), vbslq_u32(t760[0], x40[2], t758[2]), vbslq_u32(t760[1], x40[3], t758[3])];
    }
    res = t758;
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
        let t773: [uint32x4_t; 4];
        if (x8is) == 8u32 && ((x8if) as u64) + 7 * ((x8is) as u64) <= u32::MAX as u64 && all(&live) {
            let t774 = ((arg0).wrapping_add((x8b) as usize) as *const u8).add(((x8if) as usize) * 1);
            let t7750 = vld2q_u32((t774 as *const u32).add(0));
            let t7751 = vld2q_u32((t774 as *const u32).add(8));
            let t775: [uint32x4_t; 4] = [t7750.0, t7751.0, t7750.1, t7751.1];
            t773 = t775;
        } else {
            let t776: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
            let t777 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t776[0]), vandq_u32(live[1], t776[1]))));
            let t778: [uint32x4_t; 2] = [vbslq_u32(live[0], t776[0], t777), vbslq_u32(live[1], t776[1], t777)];
            let mut t779: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t778[0]) as usize * 1) as *const u64).read_unaligned(); t779[0] = vsetq_lane_u32::<0>(v as u32, t779[0]); t779[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t779[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t778[0]) as usize * 1) as *const u64).read_unaligned(); t779[0] = vsetq_lane_u32::<1>(v as u32, t779[0]); t779[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t779[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t778[0]) as usize * 1) as *const u64).read_unaligned(); t779[0] = vsetq_lane_u32::<2>(v as u32, t779[0]); t779[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t779[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t778[0]) as usize * 1) as *const u64).read_unaligned(); t779[0] = vsetq_lane_u32::<3>(v as u32, t779[0]); t779[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t779[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t778[1]) as usize * 1) as *const u64).read_unaligned(); t779[1] = vsetq_lane_u32::<0>(v as u32, t779[1]); t779[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t779[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t778[1]) as usize * 1) as *const u64).read_unaligned(); t779[1] = vsetq_lane_u32::<1>(v as u32, t779[1]); t779[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t779[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t778[1]) as usize * 1) as *const u64).read_unaligned(); t779[1] = vsetq_lane_u32::<2>(v as u32, t779[1]); t779[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t779[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t778[1]) as usize * 1) as *const u64).read_unaligned(); t779[1] = vsetq_lane_u32::<3>(v as u32, t779[1]); t779[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t779[3]); }
            t773 = t779;
        }
        res = t773;
        return res;
    }
    let t780: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
    let t781: [uint32x4_t; 2] = [t780[0], t780[1]];
    let t782: u64 = ((x8b) as usize as u64);
    let t783 = [vdupq_n_u32((t782) as u32); 2];
    let t784 = [vdupq_n_u32(((t782) >> 32) as u32); 2];
    let t785: [uint32x4_t; 2] = [vaddq_u32(t783[0], t781[0]), vaddq_u32(t783[1], t781[1])];
    let t786: [uint32x4_t; 2] = [vcltq_u32(t785[0], t783[0]), vcltq_u32(t785[1], t783[1])];
    let t787: [uint32x4_t; 4] = [t785[0], t785[1], vsubq_u32(vaddq_u32(t784[0], vdupq_n_u32(0)), t786[0]), vsubq_u32(vaddq_u32(t784[1], vdupq_n_u32(0)), t786[1])];
    let t788: u64 = !(7u64);
    let t789 = [vdupq_n_u32((t788) as u32); 2];
    let t790 = [vdupq_n_u32(((t788) >> 32) as u32); 2];
    let t791: [uint32x4_t; 4] = [vandq_u32(t787[0], t789[0]), vandq_u32(t787[1], t789[1]), vandq_u32(t787[2], t790[0]), vandq_u32(t787[3], t790[1])];
    let t792: [uint32x4_t; 4] = [live[0], live[1], live[0], live[1]];
    let t793: [uint32x4_t; 4] = [vandq_u32(t791[0], t792[0]), vandq_u32(t791[1], t792[1]), vandq_u32(t791[2], t792[2]), vandq_u32(t791[3], t792[3])];
    let t794: u64 = ((arg0) as usize as u64);
    let t795 = [vdupq_n_u32((t794) as u32); 2];
    let t796 = [vdupq_n_u32(((t794) >> 32) as u32); 2];
    let t797: [uint32x4_t; 2] = [vaddq_u32(t795[0], t793[0]), vaddq_u32(t795[1], t793[1])];
    let t798: [uint32x4_t; 2] = [vcltq_u32(t797[0], t795[0]), vcltq_u32(t797[1], t795[1])];
    let t799: [uint32x4_t; 4] = [t797[0], t797[1], vsubq_u32(vaddq_u32(t796[0], t793[2]), t798[0]), vsubq_u32(vaddq_u32(t796[1], t793[3]), t798[1])];
    x24 = t799;
    let t800: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
    let t801: [uint32x4_t; 2] = [t800[0], t800[1]];
    let t802: u64 = ((x8b) as usize as u64);
    let t803 = [vdupq_n_u32((t802) as u32); 2];
    let t804 = [vdupq_n_u32(((t802) >> 32) as u32); 2];
    let t805: [uint32x4_t; 2] = [vaddq_u32(t803[0], t801[0]), vaddq_u32(t803[1], t801[1])];
    let t806: [uint32x4_t; 2] = [vcltq_u32(t805[0], t803[0]), vcltq_u32(t805[1], t803[1])];
    let t807: [uint32x4_t; 4] = [t805[0], t805[1], vsubq_u32(vaddq_u32(t804[0], vdupq_n_u32(0)), t806[0]), vsubq_u32(vaddq_u32(t804[1], vdupq_n_u32(0)), t806[1])];
    let t808: u64 = 7u64;
    let t809 = [vdupq_n_u32((t808) as u32); 2];
    let t810: [uint32x4_t; 2] = [vandq_u32(t807[0], t809[0]), vandq_u32(t807[1], t809[1])];
    let t811 = [vdupq_n_u32(8u32); 2];
    let t812: [uint32x4_t; 2] = [vmulq_u32(t810[0], t811[0]), vmulq_u32(t810[1], t811[1])];
    x32 = t812;
    let mut t813: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t813[0] = vsetq_lane_u32::<0>(v as u32, t813[0]); t813[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t813[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t813[0] = vsetq_lane_u32::<1>(v as u32, t813[0]); t813[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t813[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t813[0] = vsetq_lane_u32::<2>(v as u32, t813[0]); t813[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t813[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t813[0] = vsetq_lane_u32::<3>(v as u32, t813[0]); t813[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t813[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t813[1] = vsetq_lane_u32::<0>(v as u32, t813[1]); t813[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t813[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t813[1] = vsetq_lane_u32::<1>(v as u32, t813[1]); t813[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t813[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t813[1] = vsetq_lane_u32::<2>(v as u32, t813[1]); t813[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t813[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t813[1] = vsetq_lane_u32::<3>(v as u32, t813[1]); t813[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t813[3]); }
    let t814: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t815: [uint32x4_t; 2] = [vandq_u32(t814[0], vdupq_n_u32(63)), vandq_u32(t814[1], vdupq_n_u32(63))];
    let t816: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t813[0], vnegq_s32(vreinterpretq_s32_u32(t815[0]))), vshlq_u32(t813[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t815[0])))), vorrq_u32(vshlq_u32(t813[1], vnegq_s32(vreinterpretq_s32_u32(t815[1]))), vshlq_u32(t813[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t815[1])))), vshlq_u32(t813[2], vnegq_s32(vreinterpretq_s32_u32(t815[0]))), vshlq_u32(t813[3], vnegq_s32(vreinterpretq_s32_u32(t815[1])))];
    x40 = t816;
    let t817 = [vdupq_n_u32(0u32); 2];
    let t818: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t817[0])), vmvnq_u32(vceqq_u32(x32[1], t817[1]))];
    let t819 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t820: [uint32x4_t; 2] = [vaddq_u32(x32[0], t819[0]), vaddq_u32(x32[1], t819[1])];
    let t821 = [vdupq_n_u32(64u32); 2];
    let t822: [uint32x4_t; 2] = [vcgtq_u32(t820[0], t821[0]), vcgtq_u32(t820[1], t821[1])];
    let t823: [uint32x4_t; 2] = [vandq_u32(t818[0], t822[0]), vandq_u32(t818[1], t822[1])];
    let mut t824: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t825: [uint32x4_t; 2] = [vandq_u32(live[0], t823[0]), vandq_u32(live[1], t823[1])];
    let t826: [uint32x4_t; 2] = [vbicq_u32(live[0], t823[0]), vbicq_u32(live[1], t823[1])];
    if any(&t825) {
        let t827: u64 = (1u64).wrapping_mul(8u64);
        let t828 = [vdupq_n_u32((t827) as u32); 2];
        let t829 = [vdupq_n_u32(((t827) >> 32) as u32); 2];
        let t830: [uint32x4_t; 2] = [vaddq_u32(x24[0], t828[0]), vaddq_u32(x24[1], t828[1])];
        let t831: [uint32x4_t; 2] = [vcltq_u32(t830[0], x24[0]), vcltq_u32(t830[1], x24[1])];
        let t832: [uint32x4_t; 4] = [t830[0], t830[1], vsubq_u32(vaddq_u32(x24[2], t829[0]), t831[0]), vsubq_u32(vaddq_u32(x24[3], t829[1]), t831[1])];
        let mut t833: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t832[2]) as u64) << 32 | vgetq_lane_u32::<0>(t832[0]) as u64) as usize as *const u64).read_unaligned(); t833[0] = vsetq_lane_u32::<0>(v as u32, t833[0]); t833[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t833[2]); }
        { let v = (((vgetq_lane_u32::<1>(t832[2]) as u64) << 32 | vgetq_lane_u32::<1>(t832[0]) as u64) as usize as *const u64).read_unaligned(); t833[0] = vsetq_lane_u32::<1>(v as u32, t833[0]); t833[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t833[2]); }
        { let v = (((vgetq_lane_u32::<2>(t832[2]) as u64) << 32 | vgetq_lane_u32::<2>(t832[0]) as u64) as usize as *const u64).read_unaligned(); t833[0] = vsetq_lane_u32::<2>(v as u32, t833[0]); t833[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t833[2]); }
        { let v = (((vgetq_lane_u32::<3>(t832[2]) as u64) << 32 | vgetq_lane_u32::<3>(t832[0]) as u64) as usize as *const u64).read_unaligned(); t833[0] = vsetq_lane_u32::<3>(v as u32, t833[0]); t833[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t833[2]); }
        { let v = (((vgetq_lane_u32::<0>(t832[3]) as u64) << 32 | vgetq_lane_u32::<0>(t832[1]) as u64) as usize as *const u64).read_unaligned(); t833[1] = vsetq_lane_u32::<0>(v as u32, t833[1]); t833[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t833[3]); }
        { let v = (((vgetq_lane_u32::<1>(t832[3]) as u64) << 32 | vgetq_lane_u32::<1>(t832[1]) as u64) as usize as *const u64).read_unaligned(); t833[1] = vsetq_lane_u32::<1>(v as u32, t833[1]); t833[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t833[3]); }
        { let v = (((vgetq_lane_u32::<2>(t832[3]) as u64) << 32 | vgetq_lane_u32::<2>(t832[1]) as u64) as usize as *const u64).read_unaligned(); t833[1] = vsetq_lane_u32::<2>(v as u32, t833[1]); t833[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t833[3]); }
        { let v = (((vgetq_lane_u32::<3>(t832[3]) as u64) << 32 | vgetq_lane_u32::<3>(t832[1]) as u64) as usize as *const u64).read_unaligned(); t833[1] = vsetq_lane_u32::<3>(v as u32, t833[1]); t833[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t833[3]); }
        let t834: [uint32x4_t; 2] = [vsubq_u32(t821[0], x32[0]), vsubq_u32(t821[1], x32[1])];
        let t835: [uint32x4_t; 2] = [t834[0], t834[1]];
        let t836: [uint32x4_t; 2] = [vandq_u32(t835[0], vdupq_n_u32(63)), vandq_u32(t835[1], vdupq_n_u32(63))];
        let t837: [uint32x4_t; 4] = [vshlq_u32(t833[0], vreinterpretq_s32_u32(t836[0])), vshlq_u32(t833[1], vreinterpretq_s32_u32(t836[1])), vorrq_u32(vshlq_u32(t833[2], vreinterpretq_s32_u32(t836[0])), vshlq_u32(t833[0], vsubq_s32(vreinterpretq_s32_u32(t836[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t833[3], vreinterpretq_s32_u32(t836[1])), vshlq_u32(t833[1], vsubq_s32(vreinterpretq_s32_u32(t836[1]), vdupq_n_s32(32))))];
        let t838: [uint32x4_t; 4] = [vorrq_u32(x40[0], t837[0]), vorrq_u32(x40[1], t837[1]), vorrq_u32(x40[2], t837[2]), vorrq_u32(x40[3], t837[3])];
        t824 = [vbslq_u32(t825[0], t838[0], t824[0]), vbslq_u32(t825[1], t838[1], t824[1]), vbslq_u32(t825[0], t838[2], t824[2]), vbslq_u32(t825[1], t838[3], t824[3])];
    }
    {
        t824 = [vbslq_u32(t826[0], x40[0], t824[0]), vbslq_u32(t826[1], x40[1], t824[1]), vbslq_u32(t826[0], x40[2], t824[2]), vbslq_u32(t826[1], x40[3], t824[3])];
    }
    res = t824;
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
        let t839: [uint32x4_t; 4];
        if (x8is) == 8u32 && ((x8if) as u64) + 7 * ((x8is) as u64) <= u32::MAX as u64 {
            let t840 = ((arg0).wrapping_add((x8b) as usize) as *const u8).add(((x8if) as usize) * 1);
            let t8410 = vld2q_u32((t840 as *const u32).add(0));
            let t8411 = vld2q_u32((t840 as *const u32).add(8));
            let t841: [uint32x4_t; 4] = [t8410.0, t8411.0, t8410.1, t8411.1];
            t839 = t841;
        } else {
            let t842: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
            let mut t843: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t842[0]) as usize * 1) as *const u64).read_unaligned(); t843[0] = vsetq_lane_u32::<0>(v as u32, t843[0]); t843[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t843[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t842[0]) as usize * 1) as *const u64).read_unaligned(); t843[0] = vsetq_lane_u32::<1>(v as u32, t843[0]); t843[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t843[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t842[0]) as usize * 1) as *const u64).read_unaligned(); t843[0] = vsetq_lane_u32::<2>(v as u32, t843[0]); t843[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t843[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t842[0]) as usize * 1) as *const u64).read_unaligned(); t843[0] = vsetq_lane_u32::<3>(v as u32, t843[0]); t843[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t843[2]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<0>(t842[1]) as usize * 1) as *const u64).read_unaligned(); t843[1] = vsetq_lane_u32::<0>(v as u32, t843[1]); t843[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t843[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<1>(t842[1]) as usize * 1) as *const u64).read_unaligned(); t843[1] = vsetq_lane_u32::<1>(v as u32, t843[1]); t843[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t843[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<2>(t842[1]) as usize * 1) as *const u64).read_unaligned(); t843[1] = vsetq_lane_u32::<2>(v as u32, t843[1]); t843[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t843[3]); }
            { let v = (((arg0).wrapping_add((x8b) as usize) as *const u8).add(vgetq_lane_u32::<3>(t842[1]) as usize * 1) as *const u64).read_unaligned(); t843[1] = vsetq_lane_u32::<3>(v as u32, t843[1]); t843[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t843[3]); }
            t839 = t843;
        }
        res = t839;
        return res;
    }
    let t844: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
    let t845: [uint32x4_t; 2] = [t844[0], t844[1]];
    let t846: u64 = ((x8b) as usize as u64);
    let t847 = [vdupq_n_u32((t846) as u32); 2];
    let t848 = [vdupq_n_u32(((t846) >> 32) as u32); 2];
    let t849: [uint32x4_t; 2] = [vaddq_u32(t847[0], t845[0]), vaddq_u32(t847[1], t845[1])];
    let t850: [uint32x4_t; 2] = [vcltq_u32(t849[0], t847[0]), vcltq_u32(t849[1], t847[1])];
    let t851: [uint32x4_t; 4] = [t849[0], t849[1], vsubq_u32(vaddq_u32(t848[0], vdupq_n_u32(0)), t850[0]), vsubq_u32(vaddq_u32(t848[1], vdupq_n_u32(0)), t850[1])];
    let t852: u64 = !(7u64);
    let t853 = [vdupq_n_u32((t852) as u32); 2];
    let t854 = [vdupq_n_u32(((t852) >> 32) as u32); 2];
    let t855: [uint32x4_t; 4] = [vandq_u32(t851[0], t853[0]), vandq_u32(t851[1], t853[1]), vandq_u32(t851[2], t854[0]), vandq_u32(t851[3], t854[1])];
    let t856: u64 = ((arg0) as usize as u64);
    let t857 = [vdupq_n_u32((t856) as u32); 2];
    let t858 = [vdupq_n_u32(((t856) >> 32) as u32); 2];
    let t859: [uint32x4_t; 2] = [vaddq_u32(t857[0], t855[0]), vaddq_u32(t857[1], t855[1])];
    let t860: [uint32x4_t; 2] = [vcltq_u32(t859[0], t857[0]), vcltq_u32(t859[1], t857[1])];
    let t861: [uint32x4_t; 4] = [t859[0], t859[1], vsubq_u32(vaddq_u32(t858[0], t855[2]), t860[0]), vsubq_u32(vaddq_u32(t858[1], t855[3]), t860[1])];
    x24 = t861;
    let t862: [uint32x4_t; 2] = [vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[0].as_ptr()), vdupq_n_u32(x8is)), vmlaq_u32(vdupq_n_u32(x8if), vld1q_u32(LANE[1].as_ptr()), vdupq_n_u32(x8is))];
    let t863: [uint32x4_t; 2] = [t862[0], t862[1]];
    let t864: u64 = ((x8b) as usize as u64);
    let t865 = [vdupq_n_u32((t864) as u32); 2];
    let t866 = [vdupq_n_u32(((t864) >> 32) as u32); 2];
    let t867: [uint32x4_t; 2] = [vaddq_u32(t865[0], t863[0]), vaddq_u32(t865[1], t863[1])];
    let t868: [uint32x4_t; 2] = [vcltq_u32(t867[0], t865[0]), vcltq_u32(t867[1], t865[1])];
    let t869: [uint32x4_t; 4] = [t867[0], t867[1], vsubq_u32(vaddq_u32(t866[0], vdupq_n_u32(0)), t868[0]), vsubq_u32(vaddq_u32(t866[1], vdupq_n_u32(0)), t868[1])];
    let t870: u64 = 7u64;
    let t871 = [vdupq_n_u32((t870) as u32); 2];
    let t872: [uint32x4_t; 2] = [vandq_u32(t869[0], t871[0]), vandq_u32(t869[1], t871[1])];
    let t873 = [vdupq_n_u32(8u32); 2];
    let t874: [uint32x4_t; 2] = [vmulq_u32(t872[0], t873[0]), vmulq_u32(t872[1], t873[1])];
    x32 = t874;
    let mut t875: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    { let v = (((vgetq_lane_u32::<0>(x24[2]) as u64) << 32 | vgetq_lane_u32::<0>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t875[0] = vsetq_lane_u32::<0>(v as u32, t875[0]); t875[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t875[2]); }
    { let v = (((vgetq_lane_u32::<1>(x24[2]) as u64) << 32 | vgetq_lane_u32::<1>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t875[0] = vsetq_lane_u32::<1>(v as u32, t875[0]); t875[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t875[2]); }
    { let v = (((vgetq_lane_u32::<2>(x24[2]) as u64) << 32 | vgetq_lane_u32::<2>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t875[0] = vsetq_lane_u32::<2>(v as u32, t875[0]); t875[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t875[2]); }
    { let v = (((vgetq_lane_u32::<3>(x24[2]) as u64) << 32 | vgetq_lane_u32::<3>(x24[0]) as u64) as usize as *const u64).read_unaligned(); t875[0] = vsetq_lane_u32::<3>(v as u32, t875[0]); t875[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t875[2]); }
    { let v = (((vgetq_lane_u32::<0>(x24[3]) as u64) << 32 | vgetq_lane_u32::<0>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t875[1] = vsetq_lane_u32::<0>(v as u32, t875[1]); t875[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t875[3]); }
    { let v = (((vgetq_lane_u32::<1>(x24[3]) as u64) << 32 | vgetq_lane_u32::<1>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t875[1] = vsetq_lane_u32::<1>(v as u32, t875[1]); t875[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t875[3]); }
    { let v = (((vgetq_lane_u32::<2>(x24[3]) as u64) << 32 | vgetq_lane_u32::<2>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t875[1] = vsetq_lane_u32::<2>(v as u32, t875[1]); t875[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t875[3]); }
    { let v = (((vgetq_lane_u32::<3>(x24[3]) as u64) << 32 | vgetq_lane_u32::<3>(x24[1]) as u64) as usize as *const u64).read_unaligned(); t875[1] = vsetq_lane_u32::<3>(v as u32, t875[1]); t875[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t875[3]); }
    let t876: [uint32x4_t; 2] = [x32[0], x32[1]];
    let t877: [uint32x4_t; 2] = [vandq_u32(t876[0], vdupq_n_u32(63)), vandq_u32(t876[1], vdupq_n_u32(63))];
    let t878: [uint32x4_t; 4] = [vorrq_u32(vshlq_u32(t875[0], vnegq_s32(vreinterpretq_s32_u32(t877[0]))), vshlq_u32(t875[2], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t877[0])))), vorrq_u32(vshlq_u32(t875[1], vnegq_s32(vreinterpretq_s32_u32(t877[1]))), vshlq_u32(t875[3], vsubq_s32(vdupq_n_s32(32), vreinterpretq_s32_u32(t877[1])))), vshlq_u32(t875[2], vnegq_s32(vreinterpretq_s32_u32(t877[0]))), vshlq_u32(t875[3], vnegq_s32(vreinterpretq_s32_u32(t877[1])))];
    x40 = t878;
    let t879 = [vdupq_n_u32(0u32); 2];
    let t880: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t879[0])), vmvnq_u32(vceqq_u32(x32[1], t879[1]))];
    let t881 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t882: [uint32x4_t; 2] = [vaddq_u32(x32[0], t881[0]), vaddq_u32(x32[1], t881[1])];
    let t883 = [vdupq_n_u32(64u32); 2];
    let t884: [uint32x4_t; 2] = [vcgtq_u32(t882[0], t883[0]), vcgtq_u32(t882[1], t883[1])];
    let t885: [uint32x4_t; 2] = [vandq_u32(t880[0], t884[0]), vandq_u32(t880[1], t884[1])];
    let mut t886: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t887: [uint32x4_t; 2] = [vmvnq_u32(t885[0]), vmvnq_u32(t885[1])];
    if any(&t885) {
        let t888: u64 = (1u64).wrapping_mul(8u64);
        let t889 = [vdupq_n_u32((t888) as u32); 2];
        let t890 = [vdupq_n_u32(((t888) >> 32) as u32); 2];
        let t891: [uint32x4_t; 2] = [vaddq_u32(x24[0], t889[0]), vaddq_u32(x24[1], t889[1])];
        let t892: [uint32x4_t; 2] = [vcltq_u32(t891[0], x24[0]), vcltq_u32(t891[1], x24[1])];
        let t893: [uint32x4_t; 4] = [t891[0], t891[1], vsubq_u32(vaddq_u32(x24[2], t890[0]), t892[0]), vsubq_u32(vaddq_u32(x24[3], t890[1]), t892[1])];
        let mut t894: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
        { let v = (((vgetq_lane_u32::<0>(t893[2]) as u64) << 32 | vgetq_lane_u32::<0>(t893[0]) as u64) as usize as *const u64).read_unaligned(); t894[0] = vsetq_lane_u32::<0>(v as u32, t894[0]); t894[2] = vsetq_lane_u32::<0>((v >> 32) as u32, t894[2]); }
        { let v = (((vgetq_lane_u32::<1>(t893[2]) as u64) << 32 | vgetq_lane_u32::<1>(t893[0]) as u64) as usize as *const u64).read_unaligned(); t894[0] = vsetq_lane_u32::<1>(v as u32, t894[0]); t894[2] = vsetq_lane_u32::<1>((v >> 32) as u32, t894[2]); }
        { let v = (((vgetq_lane_u32::<2>(t893[2]) as u64) << 32 | vgetq_lane_u32::<2>(t893[0]) as u64) as usize as *const u64).read_unaligned(); t894[0] = vsetq_lane_u32::<2>(v as u32, t894[0]); t894[2] = vsetq_lane_u32::<2>((v >> 32) as u32, t894[2]); }
        { let v = (((vgetq_lane_u32::<3>(t893[2]) as u64) << 32 | vgetq_lane_u32::<3>(t893[0]) as u64) as usize as *const u64).read_unaligned(); t894[0] = vsetq_lane_u32::<3>(v as u32, t894[0]); t894[2] = vsetq_lane_u32::<3>((v >> 32) as u32, t894[2]); }
        { let v = (((vgetq_lane_u32::<0>(t893[3]) as u64) << 32 | vgetq_lane_u32::<0>(t893[1]) as u64) as usize as *const u64).read_unaligned(); t894[1] = vsetq_lane_u32::<0>(v as u32, t894[1]); t894[3] = vsetq_lane_u32::<0>((v >> 32) as u32, t894[3]); }
        { let v = (((vgetq_lane_u32::<1>(t893[3]) as u64) << 32 | vgetq_lane_u32::<1>(t893[1]) as u64) as usize as *const u64).read_unaligned(); t894[1] = vsetq_lane_u32::<1>(v as u32, t894[1]); t894[3] = vsetq_lane_u32::<1>((v >> 32) as u32, t894[3]); }
        { let v = (((vgetq_lane_u32::<2>(t893[3]) as u64) << 32 | vgetq_lane_u32::<2>(t893[1]) as u64) as usize as *const u64).read_unaligned(); t894[1] = vsetq_lane_u32::<2>(v as u32, t894[1]); t894[3] = vsetq_lane_u32::<2>((v >> 32) as u32, t894[3]); }
        { let v = (((vgetq_lane_u32::<3>(t893[3]) as u64) << 32 | vgetq_lane_u32::<3>(t893[1]) as u64) as usize as *const u64).read_unaligned(); t894[1] = vsetq_lane_u32::<3>(v as u32, t894[1]); t894[3] = vsetq_lane_u32::<3>((v >> 32) as u32, t894[3]); }
        let t895: [uint32x4_t; 2] = [vsubq_u32(t883[0], x32[0]), vsubq_u32(t883[1], x32[1])];
        let t896: [uint32x4_t; 2] = [t895[0], t895[1]];
        let t897: [uint32x4_t; 2] = [vandq_u32(t896[0], vdupq_n_u32(63)), vandq_u32(t896[1], vdupq_n_u32(63))];
        let t898: [uint32x4_t; 4] = [vshlq_u32(t894[0], vreinterpretq_s32_u32(t897[0])), vshlq_u32(t894[1], vreinterpretq_s32_u32(t897[1])), vorrq_u32(vshlq_u32(t894[2], vreinterpretq_s32_u32(t897[0])), vshlq_u32(t894[0], vsubq_s32(vreinterpretq_s32_u32(t897[0]), vdupq_n_s32(32)))), vorrq_u32(vshlq_u32(t894[3], vreinterpretq_s32_u32(t897[1])), vshlq_u32(t894[1], vsubq_s32(vreinterpretq_s32_u32(t897[1]), vdupq_n_s32(32))))];
        let t899: [uint32x4_t; 4] = [vorrq_u32(x40[0], t898[0]), vorrq_u32(x40[1], t898[1]), vorrq_u32(x40[2], t898[2]), vorrq_u32(x40[3], t898[3])];
        t886 = [vbslq_u32(t885[0], t899[0], t886[0]), vbslq_u32(t885[1], t899[1], t886[1]), vbslq_u32(t885[0], t899[2], t886[2]), vbslq_u32(t885[1], t899[3], t886[3])];
    }
    {
        t886 = [vbslq_u32(t887[0], x40[0], t886[0]), vbslq_u32(t887[1], x40[1], t886[1]), vbslq_u32(t887[0], x40[2], t886[2]), vbslq_u32(t887[1], x40[3], t886[3])];
    }
    res = t886;
    return res;
}

#[inline(always)]
unsafe fn b18_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t900: u64;
    if ((x0) >= (64u32)) {
        t900 = !(0u64);
    } else {
        t900 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t900;
    return res;
}

#[inline(always)]
unsafe fn b18_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t901: u64;
    if ((x0) >= (64u32)) {
        t901 = !(0u64);
    } else {
        t901 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t901;
    return res;
}

#[inline(always)]
unsafe fn b19_r0sss_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16: u32, x24: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u64 = 0;
    let mut live = k;
    let mut res: u64 = 0;
    let t902 = b17_r0ss_p(l, lid0, live, arg0, (((x8) as u64)).wrapping_add(((x16) as u64)), x24);
    x32 = t902;
    let t903: u64;
    if ((x24) >= (8u32)) {
        t903 = x32;
    } else {
        let t904 = b18_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        t903 = ((x32) & (t904));
    }
    res = t903;
    return res;
}

#[inline(always)]
unsafe fn b19_r0sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16: [uint32x4_t; 2], x24: u32) -> [uint32x4_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 4] = [vdupq_n_u32(0); 4];
    let t905 = b17_r0al1s_p(l, lid0, live, arg0, ((((x8) as u64)) as usize as *const u8), x16, x24);
    x32 = t905;
    let t906: [uint32x4_t; 4];
    if ((x24) >= (8u32)) {
        t906 = x32;
    } else {
        let t907 = b18_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t908: u64 = t907;
        let t909 = [vdupq_n_u32((t908) as u32); 2];
        let t910 = [vdupq_n_u32(((t908) >> 32) as u32); 2];
        let t911: [uint32x4_t; 4] = [vandq_u32(x32[0], t909[0]), vandq_u32(x32[1], t909[1]), vandq_u32(x32[2], t910[0]), vandq_u32(x32[3], t910[1])];
        t906 = t911;
    }
    res = t906;
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
    let mut t912: bool = !(arg4).is_null();
    if t912 {
        let mut t913: bool = ((l.group[0]) >= ((arg4.wrapping_add((((x48) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t913 {
            t913 = ((l.group[1]) >= ((arg4.wrapping_add(((((x48).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t914: bool = t913;
        if !t914 {
            t914 = ((l.group[2]) >= ((arg4.wrapping_add(((((x48).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t912 = t914;
    }
    if t912 {
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
        let t915: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t915;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1r2sg_p(l, lid0, live, arg0, arg1, arg2, x24);
        }
    }
}
