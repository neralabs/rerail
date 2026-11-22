//! `parent_all` at 8 lanes (2 NEON registers), written by rerail-ukl-vec's Rust (`rs.rs`) from its vector tape: do
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
    let mut live = k;
    let t1: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t2 = [vdupq_n_u32(a0); 2];
    let t3: [uint32x4_t; 2] = [vcgeq_u32(t1[0], t2[0]), vcgeq_u32(t1[1], t2[1])];
    let t4: [uint32x4_t; 2] = [vandq_u32(live[0], t3[0]), vandq_u32(live[1], t3[1])];
    if any(&t4) {
        live = [vbicq_u32(live[0], t4[0]), vbicq_u32(live[1], t4[1])];
    }
    if any(&live) {
        let t5 = b1_r0r1g_p(l, lid0, live, arg0, arg1);
        let t6 = (arg2 as *mut u32).add((gid0) as usize);
        if all(&live) {
            vst1q_u32(t6.add(0), t5[0]);
            vst1q_u32(t6.add(4), t5[1]);
        } else {
            let mut t7w = 0u32;
            let t7 = core::hint::black_box(&raw mut t7w);
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t6.add(0), t7).write(vgetq_lane_u32::<0>(t5[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t6.add(1), t7).write(vgetq_lane_u32::<1>(t5[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t6.add(2), t7).write(vgetq_lane_u32::<2>(t5[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t6.add(3), t7).write(vgetq_lane_u32::<3>(t5[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t6.add(4), t7).write(vgetq_lane_u32::<0>(t5[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t6.add(5), t7).write(vgetq_lane_u32::<1>(t5[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t6.add(6), t7).write(vgetq_lane_u32::<2>(t5[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t6.add(7), t7).write(vgetq_lane_u32::<3>(t5[1]));
        }
    }
}

#[inline(always)]
unsafe fn b0_r0r1r2sg_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, arg2: *const u8, x24: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x24 as *const u32).read_unaligned();
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if ((gid0) as u64 + 7) < ((a0) as u64) {
        let t8 = b1_r0r1g_f(l, lid0, arg0, arg1);
        let t9 = (arg2 as *mut u32).add((gid0) as usize);
        vst1q_u32(t9.add(0), t8[0]);
        vst1q_u32(t9.add(4), t8[1]);
    } else if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
        return;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t10: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t11 = [vdupq_n_u32(a0); 2];
        let t12: [uint32x4_t; 2] = [vcgeq_u32(t10[0], t11[0]), vcgeq_u32(t10[1], t11[1])];
        let t13: [uint32x4_t; 2] = [vandq_u32(live[0], t12[0]), vandq_u32(live[1], t12[1])];
        if any(&t13) {
            live = [vbicq_u32(live[0], t13[0]), vbicq_u32(live[1], t13[1])];
        }
        if any(&live) {
            let t14 = b1_r0r1g_p(l, lid0, live, arg0, arg1);
            let t15 = (arg2 as *mut u32).add((gid0) as usize);
            if all(&live) {
                vst1q_u32(t15.add(0), t14[0]);
                vst1q_u32(t15.add(4), t14[1]);
            } else {
                let mut t16w = 0u32;
                let t16 = core::hint::black_box(&raw mut t16w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t15.add(0), t16).write(vgetq_lane_u32::<0>(t14[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t15.add(1), t16).write(vgetq_lane_u32::<1>(t14[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t15.add(2), t16).write(vgetq_lane_u32::<2>(t14[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t15.add(3), t16).write(vgetq_lane_u32::<3>(t14[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t15.add(4), t16).write(vgetq_lane_u32::<0>(t14[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t15.add(5), t16).write(vgetq_lane_u32::<1>(t14[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t15.add(6), t16).write(vgetq_lane_u32::<2>(t14[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t15.add(7), t16).write(vgetq_lane_u32::<3>(t14[1]));
            }
        }
    }
}

#[inline(always)]
unsafe fn b1_r0r1g_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t17: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    x24 = t17;
    let mut t18 = live;
    loop {
        // Lanes part: each its own rounds.
        let t20 = [vdupq_n_u32(0u32); 2];
        let t21: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x24[0], t20[0])), vmvnq_u32(vceqq_u32(x24[1], t20[1]))];
        t18 = [vandq_u32(t18[0], t21[0]), vandq_u32(t18[1], t21[1])];
        if !(any(&t18)) { break; }
        let t23 = [vdupq_n_u32(1u32); 2];
        let t24: [uint32x4_t; 2] = [vsubq_u32(x24[0], t23[0]), vsubq_u32(x24[1], t23[1])];
        x32 = t24;
        let t25 = b2_r0r1l_p(l, lid0, t18, arg0, arg1, x32);
        x40 = t25;
        let t26 = [vdupq_n_u64(1u64); 4];
        let t27: [uint64x2_t; 4] = [vandq_u64(x40[0], t26[0]), vandq_u64(x40[1], t26[1]), vandq_u64(x40[2], t26[2]), vandq_u64(x40[3], t26[3])];
        let t28 = [vdupq_n_u64(0u64); 4];
        let t29: [uint64x2_t; 4] = [vceqq_u64(t27[0], t28[0]), vceqq_u64(t27[1], t28[1]), vceqq_u64(t27[2], t28[2]), vceqq_u64(t27[3], t28[3])];
        let t30: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t29[0]), vmovn_u64(t29[1])), vcombine_u32(vmovn_u64(t29[2]), vmovn_u64(t29[3]))];
        let t31: [uint32x4_t; 2] = [vandq_u32(t18[0], t30[0]), vandq_u32(t18[1], t30[1])];
        if any(&t31) {
            res = [vbslq_u32(t31[0], x32[0], res[0]), vbslq_u32(t31[1], x32[1], res[1])];
            live = [vbicq_u32(live[0], t31[0]), vbicq_u32(live[1], t31[1])];
        }
        let t32: [uint32x4_t; 2] = [vandq_u32(t18[0], live[0]), vandq_u32(t18[1], live[1])];
        if any(&t32) {
            let t33: [uint64x2_t; 4] = [vshlq_u64(x40[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x40[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x40[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x40[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
            let t34: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t33[0]), vmovn_u64(t33[1])), vcombine_u32(vmovn_u64(t33[2]), vmovn_u64(t33[3]))];
            x48 = t34;
            let t35: [uint32x4_t; 2] = [vsubq_u32(x48[0], t23[0]), vsubq_u32(x48[1], t23[1])];
            let t36: [uint32x4_t; 2] = [vcgeq_u32(t35[0], x24[0]), vcgeq_u32(t35[1], x24[1])];
            let t37: [uint32x4_t; 2] = [vandq_u32(t32[0], t36[0]), vandq_u32(t32[1], t36[1])];
            if any(&t37) {
                let t38 = [vdupq_n_u32(4294967295u32); 2];
                res = [vbslq_u32(t37[0], t38[0], res[0]), vbslq_u32(t37[1], t38[1], res[1])];
                live = [vbicq_u32(live[0], t37[0]), vbicq_u32(live[1], t37[1])];
            }
            let t39: [uint32x4_t; 2] = [vandq_u32(t32[0], live[0]), vandq_u32(t32[1], live[1])];
            if any(&t39) {
                let t40: [uint32x4_t; 2] = [vsubq_u32(x24[0], x48[0]), vsubq_u32(x24[1], x48[1])];
                x24 = t40;
                let t41 = [vdupq_n_u64(2u64); 4];
                let t42: [uint64x2_t; 4] = [vandq_u64(x40[0], t41[0]), vandq_u64(x40[1], t41[1]), vandq_u64(x40[2], t41[2]), vandq_u64(x40[3], t41[3])];
                let t43: [uint64x2_t; 4] = [vceqq_u64(t42[0], t28[0]), vceqq_u64(t42[1], t28[1]), vceqq_u64(t42[2], t28[2]), vceqq_u64(t42[3], t28[3])];
                let t44: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t43[0]), vmovn_u64(t43[1])), vcombine_u32(vmovn_u64(t43[2]), vmovn_u64(t43[3]))];
                let t45: [uint32x4_t; 2] = [vmvnq_u32(t44[0]), vmvnq_u32(t44[1])];
                let t46: [uint32x4_t; 2] = [vandq_u32(t39[0], t45[0]), vandq_u32(t39[1], t45[1])];
                if any(&t46) {
                    res = [vbslq_u32(t46[0], x24[0], res[0]), vbslq_u32(t46[1], x24[1], res[1])];
                    live = [vbicq_u32(live[0], t46[0]), vbicq_u32(live[1], t46[1])];
                }
            }
        }
        t18 = [vandq_u32(t18[0], live[0]), vandq_u32(t18[1], live[1])];
    }
    if any(&live) {
        let t47 = [vdupq_n_u32(4294967295u32); 2];
        res = [vbslq_u32(live[0], t47[0], res[0]), vbslq_u32(live[1], t47[1], res[1])];
        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
    }
    res
}

#[inline(always)]
unsafe fn b1_r0r1g_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t48: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    x24 = t48;
    let mut t49 = live;
    loop {
        // Lanes part: each its own rounds.
        let t51 = [vdupq_n_u32(0u32); 2];
        let t52: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x24[0], t51[0])), vmvnq_u32(vceqq_u32(x24[1], t51[1]))];
        t49 = [vandq_u32(t49[0], t52[0]), vandq_u32(t49[1], t52[1])];
        if !(any(&t49)) { break; }
        let t54 = [vdupq_n_u32(1u32); 2];
        let t55: [uint32x4_t; 2] = [vsubq_u32(x24[0], t54[0]), vsubq_u32(x24[1], t54[1])];
        x32 = t55;
        let t56 = b2_r0r1l_p(l, lid0, t49, arg0, arg1, x32);
        x40 = t56;
        let t57 = [vdupq_n_u64(1u64); 4];
        let t58: [uint64x2_t; 4] = [vandq_u64(x40[0], t57[0]), vandq_u64(x40[1], t57[1]), vandq_u64(x40[2], t57[2]), vandq_u64(x40[3], t57[3])];
        let t59 = [vdupq_n_u64(0u64); 4];
        let t60: [uint64x2_t; 4] = [vceqq_u64(t58[0], t59[0]), vceqq_u64(t58[1], t59[1]), vceqq_u64(t58[2], t59[2]), vceqq_u64(t58[3], t59[3])];
        let t61: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t60[0]), vmovn_u64(t60[1])), vcombine_u32(vmovn_u64(t60[2]), vmovn_u64(t60[3]))];
        let t62: [uint32x4_t; 2] = [vandq_u32(t49[0], t61[0]), vandq_u32(t49[1], t61[1])];
        if any(&t62) {
            res = [vbslq_u32(t62[0], x32[0], res[0]), vbslq_u32(t62[1], x32[1], res[1])];
            live = [vbicq_u32(live[0], t62[0]), vbicq_u32(live[1], t62[1])];
        }
        let t63: [uint32x4_t; 2] = [vandq_u32(t49[0], live[0]), vandq_u32(t49[1], live[1])];
        if any(&t63) {
            let t64: [uint64x2_t; 4] = [vshlq_u64(x40[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x40[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x40[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x40[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
            let t65: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t64[0]), vmovn_u64(t64[1])), vcombine_u32(vmovn_u64(t64[2]), vmovn_u64(t64[3]))];
            x48 = t65;
            let t66: [uint32x4_t; 2] = [vsubq_u32(x48[0], t54[0]), vsubq_u32(x48[1], t54[1])];
            let t67: [uint32x4_t; 2] = [vcgeq_u32(t66[0], x24[0]), vcgeq_u32(t66[1], x24[1])];
            let t68: [uint32x4_t; 2] = [vandq_u32(t63[0], t67[0]), vandq_u32(t63[1], t67[1])];
            if any(&t68) {
                let t69 = [vdupq_n_u32(4294967295u32); 2];
                res = [vbslq_u32(t68[0], t69[0], res[0]), vbslq_u32(t68[1], t69[1], res[1])];
                live = [vbicq_u32(live[0], t68[0]), vbicq_u32(live[1], t68[1])];
            }
            let t70: [uint32x4_t; 2] = [vandq_u32(t63[0], live[0]), vandq_u32(t63[1], live[1])];
            if any(&t70) {
                let t71: [uint32x4_t; 2] = [vsubq_u32(x24[0], x48[0]), vsubq_u32(x24[1], x48[1])];
                x24 = t71;
                let t72 = [vdupq_n_u64(2u64); 4];
                let t73: [uint64x2_t; 4] = [vandq_u64(x40[0], t72[0]), vandq_u64(x40[1], t72[1]), vandq_u64(x40[2], t72[2]), vandq_u64(x40[3], t72[3])];
                let t74: [uint64x2_t; 4] = [vceqq_u64(t73[0], t59[0]), vceqq_u64(t73[1], t59[1]), vceqq_u64(t73[2], t59[2]), vceqq_u64(t73[3], t59[3])];
                let t75: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t74[0]), vmovn_u64(t74[1])), vcombine_u32(vmovn_u64(t74[2]), vmovn_u64(t74[3]))];
                let t76: [uint32x4_t; 2] = [vmvnq_u32(t75[0]), vmvnq_u32(t75[1])];
                let t77: [uint32x4_t; 2] = [vandq_u32(t70[0], t76[0]), vandq_u32(t70[1], t76[1])];
                if any(&t77) {
                    res = [vbslq_u32(t77[0], x24[0], res[0]), vbslq_u32(t77[1], x24[1], res[1])];
                    live = [vbicq_u32(live[0], t77[0]), vbicq_u32(live[1], t77[1])];
                }
            }
        }
        t49 = [vandq_u32(t49[0], live[0]), vandq_u32(t49[1], live[1])];
    }
    if any(&live) {
        let t78 = [vdupq_n_u32(4294967295u32); 2];
        res = [vbslq_u32(live[0], t78[0], res[0]), vbslq_u32(live[1], t78[1], res[1])];
        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
    }
    res
}

#[inline(always)]
unsafe fn b2_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t79 = b3_r0r1sl_p(l, lid0, live, arg0, arg1, 0u32, x16);
    res = t79;
    return res;
}

#[inline(always)]
unsafe fn b3_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: u64 = 0;
    let mut x56: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t80 = b4_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    x32 = t80;
    let t81: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t82 = b5_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x16), t81);
    x40 = t82;
    let t83 = b6_s_p(l, lid0, live, x32);
    x48 = t83;
    let t84 = [vdupq_n_u32((1024u32).wrapping_sub(1u32)); 2];
    let t85: [uint32x4_t; 2] = [vandq_u32(x24[0], t84[0]), vandq_u32(x24[1], t84[1])];
    let t86 = [vdupq_n_u32(x32); 2];
    let t87: [uint32x4_t; 2] = [vmulq_u32(t85[0], t86[0]), vmulq_u32(t85[1], t86[1])];
    let t88 = b7_r0r1lls_p(l, lid0, live, arg0, arg1, x40, t87, 8u32);
    let t89 = [vdupq_n_u64(x48); 4];
    let t90: [uint64x2_t; 4] = [vandq_u64(t88[0], t89[0]), vandq_u64(t88[1], t89[1]), vandq_u64(t88[2], t89[2]), vandq_u64(t88[3], t89[3])];
    x56 = t90;
    let t91 = [vdupq_n_u64(x48); 4];
    let t92: [uint64x2_t; 4] = [vceqq_u64(x56[0], t91[0]), vceqq_u64(x56[1], t91[1]), vceqq_u64(x56[2], t91[2]), vceqq_u64(x56[3], t91[3])];
    let t93: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t92[0]), vmovn_u64(t92[1])), vcombine_u32(vmovn_u64(t92[2]), vmovn_u64(t92[3]))];
    let mut t94: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t95: [uint32x4_t; 2] = [vandq_u32(live[0], t93[0]), vandq_u32(live[1], t93[1])];
    let t96: [uint32x4_t; 2] = [vbicq_u32(live[0], t93[0]), vbicq_u32(live[1], t93[1])];
    if any(&t95) {
        let t97 = b8_r0r1sl_p(l, lid0, t95, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t94 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t95[0])))), t97[0], t94[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t95[0])))), t97[1], t94[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t95[1])))), t97[2], t94[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t95[1])))), t97[3], t94[3])];
    }
    {
        t94 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t96[0])))), x56[0], t94[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t96[0])))), x56[1], t94[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t96[1])))), x56[2], t94[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t96[1])))), x56[3], t94[3])];
    }
    res = t94;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t98 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    res = (((t98).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t99 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t100 = [vdupq_n_u32(t99); 2];
    let t101: [uint32x4_t; 2] = [vmulq_u32(x24[0], t100[0]), vmulq_u32(x24[1], t100[1])];
    let t102 = b10_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    let t103 = [vdupq_n_u32(t102); 2];
    let t104: [uint32x4_t; 2] = [vaddq_u32(t101[0], t103[0]), vaddq_u32(t101[1], t103[1])];
    x32 = t104;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t105 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t106: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t107 = [vdupq_n_u32(t105); 2];
    let t108: [uint32x4_t; 2] = [vsubq_u32(t107[0], t106[0]), vsubq_u32(t107[1], t106[1])];
    let t109 = b11_ls_p(l, lid0, live, t108, 1024u32);
    x40 = t109;
    let t110 = b10_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    let t111 = [vdupq_n_u32(t110); 2];
    let t112: [uint32x4_t; 2] = [vmulq_u32(x40[0], t111[0]), vmulq_u32(x40[1], t111[1])];
    let t113: [uint32x4_t; 2] = [vaddq_u32(x32[0], t112[0]), vaddq_u32(x32[1], t112[1])];
    res = t113;
    return res;
}

#[inline(always)]
unsafe fn b6_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t114: u64;
    if ((x0) >= (8u32)) {
        t114 = !(0u64);
    } else {
        t114 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t114;
    return res;
}

#[inline(always)]
unsafe fn b6_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t115 = [vdupq_n_u32(8u32); 2];
    let t116: [uint32x4_t; 2] = [vcgeq_u32(x0[0], t115[0]), vcgeq_u32(x0[1], t115[1])];
    let mut t117: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t118: [uint32x4_t; 2] = [vandq_u32(live[0], t116[0]), vandq_u32(live[1], t116[1])];
    let t119: [uint32x4_t; 2] = [vbicq_u32(live[0], t116[0]), vbicq_u32(live[1], t116[1])];
    {
        let t120 = [vdupq_n_u64(!(0u64)); 4];
        t117 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t118[0])))), t120[0], t117[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t118[0])))), t120[1], t117[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t118[1])))), t120[2], t117[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t118[1])))), t120[3], t117[3])];
    }
    {
        let t121: [uint32x4_t; 2] = [vmulq_u32(x0[0], t115[0]), vmulq_u32(x0[1], t115[1])];
        let t122 = [vdupq_n_u64(1u64); 4];
        let t123: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t121[0])), vmovl_u32(vget_high_u32(t121[0])), vmovl_u32(vget_low_u32(t121[1])), vmovl_u32(vget_high_u32(t121[1]))];
        let t124: [uint64x2_t; 4] = [vshlq_u64(t122[0], vreinterpretq_s64_u64(vandq_u64(t123[0], vdupq_n_u64(63)))), vshlq_u64(t122[1], vreinterpretq_s64_u64(vandq_u64(t123[1], vdupq_n_u64(63)))), vshlq_u64(t122[2], vreinterpretq_s64_u64(vandq_u64(t123[2], vdupq_n_u64(63)))), vshlq_u64(t122[3], vreinterpretq_s64_u64(vandq_u64(t123[3], vdupq_n_u64(63))))];
        let t125: [uint64x2_t; 4] = [vsubq_u64(t124[0], t122[0]), vsubq_u64(t124[1], t122[1]), vsubq_u64(t124[2], t122[2]), vsubq_u64(t124[3], t122[3])];
        t117 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t119[0])))), t125[0], t117[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t119[0])))), t125[1], t117[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t119[1])))), t125[2], t117[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t119[1])))), t125[3], t117[3])];
    }
    res = t117;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t126 = b12_r0lls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t126;
    return res;
}

#[inline(always)]
unsafe fn b8_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
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
    let mut x128: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x136: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x144: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t127 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t127;
    let t128 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t128;
    let t129: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x48 = t129;
    let t130 = [vdupq_n_u32(4u32); 2];
    let t131: [uint32x4_t; 2] = [vmulq_u32(x48[0], t130[0]), vmulq_u32(x48[1], t130[1])];
    let t132 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t131, 4u32);
    let t133: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t132[0]), vmovn_u64(t132[1])), vcombine_u32(vmovn_u64(t132[2]), vmovn_u64(t132[3]))];
    x56 = t133;
    let t134: [uint32x4_t; 2] = [vmulq_u32(x48[0], t130[0]), vmulq_u32(x48[1], t130[1])];
    let t135: [uint32x4_t; 2] = [vaddq_u32(t134[0], t130[0]), vaddq_u32(t134[1], t130[1])];
    let t136 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t135, 4u32);
    let t137: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t136[0]), vmovn_u64(t136[1])), vcombine_u32(vmovn_u64(t136[2]), vmovn_u64(t136[3]))];
    x64 = t137;
    let t138 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x72 = ((t138).wrapping_add((256u32).wrapping_sub(1u32))).wrapping_shr((8u32) as u32);
    let t139: [uint32x4_t; 2] = [vmulq_u32(x48[0], t130[0]), vmulq_u32(x48[1], t130[1])];
    let t140 = [vdupq_n_u32(x72); 2];
    let t141: [uint32x4_t; 2] = [vsubq_u32(t140[0], t139[0]), vsubq_u32(t140[1], t139[1])];
    let t142 = b14_sl_p(l, lid0, live, 4u32, t141);
    x80 = t142;
    let t143: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t144 = [vdupq_n_u32((4u32).wrapping_sub(1u32)); 2];
    let t145: [uint32x4_t; 2] = [vandq_u32(t143[0], t144[0]), vandq_u32(t143[1], t144[1])];
    x88 = t145;
    let t146 = [vdupq_n_u32(0u32); 2];
    let t147: [uint32x4_t; 2] = [vceqq_u32(x88[0], t146[0]), vceqq_u32(x88[1], t146[1])];
    let mut t148: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t149: [uint32x4_t; 2] = [vandq_u32(live[0], t147[0]), vandq_u32(live[1], t147[1])];
    let t150: [uint32x4_t; 2] = [vbicq_u32(live[0], t147[0]), vbicq_u32(live[1], t147[1])];
    {
        t148 = [vbslq_u32(t149[0], t146[0], t148[0]), vbslq_u32(t149[1], t146[1], t148[1])];
    }
    if any(&t150) {
        let t151 = [vdupq_n_u32(1u32); 2];
        let t152: [uint32x4_t; 2] = [vsubq_u32(x88[0], t151[0]), vsubq_u32(x88[1], t151[1])];
        let t153 = [vdupq_n_u32(2u32); 2];
        let t154: [uint32x4_t; 2] = [vmulq_u32(t153[0], t152[0]), vmulq_u32(t153[1], t152[1])];
        let t155: [uint32x4_t; 2] = [vaddq_u32(x56[0], t154[0]), vaddq_u32(x56[1], t154[1])];
        let t156 = b15_ll_p(l, lid0, t150, t155, x64);
        let t157 = b13_r0r1sls_p(l, lid0, t150, arg0, arg1, x32, t156, 2u32);
        let t158: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t157[0]), vmovn_u64(t157[1])), vcombine_u32(vmovn_u64(t157[2]), vmovn_u64(t157[3]))];
        t148 = [vbslq_u32(t150[0], t158[0], t148[0]), vbslq_u32(t150[1], t158[1], t148[1])];
    }
    x96 = t148;
    let t159 = [vdupq_n_u32(2u32); 2];
    let t160: [uint32x4_t; 2] = [vmulq_u32(t159[0], x88[0]), vmulq_u32(t159[1], x88[1])];
    let t161: [uint32x4_t; 2] = [vaddq_u32(x56[0], t160[0]), vaddq_u32(x56[1], t160[1])];
    let t162 = b15_ll_p(l, lid0, live, t161, x64);
    let t163 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t162, 2u32);
    let t164: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t163[0]), vmovn_u64(t163[1])), vcombine_u32(vmovn_u64(t163[2]), vmovn_u64(t163[3]))];
    x104 = t164;
    let t165: [uint32x4_t; 2] = [vmulq_u32(t159[0], x80[0]), vmulq_u32(t159[1], x80[1])];
    let t166: [uint32x4_t; 2] = [vaddq_u32(x56[0], t165[0]), vaddq_u32(x56[1], t165[1])];
    x112 = t166;
    let t167 = [vdupq_n_u32((256u32).wrapping_sub(1u32)); 2];
    let t168: [uint32x4_t; 2] = [vandq_u32(x24[0], t167[0]), vandq_u32(x24[1], t167[1])];
    x120 = t168;
    let mut t169 = live;
    loop {
        // Lanes part: each its own rounds.
        let t171: [uint32x4_t; 2] = [vcltq_u32(x96[0], x104[0]), vcltq_u32(x96[1], x104[1])];
        t169 = [vandq_u32(t169[0], t171[0]), vandq_u32(t169[1], t171[1])];
        if !(any(&t169)) { break; }
        let t173: [uint32x4_t; 2] = [vaddq_u32(x96[0], x104[0]), vaddq_u32(x96[1], x104[1])];
        let t174: [uint32x4_t; 2] = [vshlq_u32(t173[0], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32))), vshlq_u32(t173[1], vnegq_s32(vdupq_n_s32(((1u32) & 31) as i32)))];
        x128 = [vbslq_u32(t169[0], t174[0], x128[0]), vbslq_u32(t169[1], t174[1], x128[1])];
        let t175: [uint32x4_t; 2] = [vaddq_u32(x112[0], x128[0]), vaddq_u32(x112[1], x128[1])];
        let t176 = b15_ll_p(l, lid0, t169, t175, x64);
        let t177 = b13_r0r1sls_p(l, lid0, t169, arg0, arg1, x32, t176, 1u32);
        let t178: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t177[0]), vmovn_u64(t177[1])), vcombine_u32(vmovn_u64(t177[2]), vmovn_u64(t177[3]))];
        let t179: [uint32x4_t; 2] = [vcltq_u32(t178[0], x120[0]), vcltq_u32(t178[1], x120[1])];
        let t180: [uint32x4_t; 2] = [vandq_u32(t169[0], t179[0]), vandq_u32(t169[1], t179[1])];
        {
            let t181 = [vdupq_n_u32(1u32); 2];
            let t182: [uint32x4_t; 2] = [vaddq_u32(x128[0], t181[0]), vaddq_u32(x128[1], t181[1])];
            x96 = [vbslq_u32(t180[0], t182[0], x96[0]), vbslq_u32(t180[1], t182[1], x96[1])];
        }
        let t183: [uint32x4_t; 2] = [vbicq_u32(t169[0], t179[0]), vbicq_u32(t169[1], t179[1])];
        {
            x104 = [vbslq_u32(t183[0], x128[0], x104[0]), vbslq_u32(t183[1], x128[1], x104[1])];
        }
    }
    let t184 = b9_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    let t185 = [vdupq_n_u32((((t184).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32))); 2];
    x128 = t185;
    let t186 = [vdupq_n_u32(1u32); 2];
    let t187: [uint32x4_t; 2] = [vsubq_u32(x80[0], t186[0]), vsubq_u32(x80[1], t186[1])];
    let t188: [uint32x4_t; 2] = [vmulq_u32(t159[0], t187[0]), vmulq_u32(t159[1], t187[1])];
    let t189: [uint32x4_t; 2] = [vaddq_u32(x56[0], t188[0]), vaddq_u32(x56[1], t188[1])];
    let t190 = b15_ll_p(l, lid0, live, t189, x64);
    let t191 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t190, 2u32);
    let t192: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t191[0]), vmovn_u64(t191[1])), vcombine_u32(vmovn_u64(t191[2]), vmovn_u64(t191[3]))];
    x136 = t192;
    let t193: [uint32x4_t; 2] = [vaddq_u32(x112[0], x136[0]), vaddq_u32(x112[1], x136[1])];
    let t194: [uint32x4_t; 2] = [vmulq_u32(x96[0], x128[0]), vmulq_u32(x96[1], x128[1])];
    let t195: [uint32x4_t; 2] = [vaddq_u32(t193[0], t194[0]), vaddq_u32(t193[1], t194[1])];
    let t196 = b15_ll_p(l, lid0, live, t195, x64);
    x144 = t196;
    let t197 = b13_r0r1sls_p(l, lid0, live, arg0, arg1, x32, x144, 8u32);
    let t198 = b6_l_p(l, lid0, live, x128);
    let t199: [uint64x2_t; 4] = [vandq_u64(t197[0], t198[0]), vandq_u64(t197[1], t198[1]), vandq_u64(t197[2], t198[2]), vandq_u64(t197[3], t198[3])];
    res = t199;
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
unsafe fn b10_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t200 = b9_r0r1s_p(l, lid0, live, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t200;
    return res;
}

#[inline(always)]
unsafe fn b11_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t201 = [vdupq_n_u32(x8); 2];
    let t202: [uint32x4_t; 2] = [vcltq_u32(x0[0], t201[0]), vcltq_u32(x0[1], t201[1])];
    let mut t203: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t204: [uint32x4_t; 2] = [vandq_u32(live[0], t202[0]), vandq_u32(live[1], t202[1])];
    let t205: [uint32x4_t; 2] = [vbicq_u32(live[0], t202[0]), vbicq_u32(live[1], t202[1])];
    {
        t203 = [vbslq_u32(t204[0], x0[0], t203[0]), vbslq_u32(t204[1], x0[1], t203[1])];
    }
    {
        let t206 = [vdupq_n_u32(x8); 2];
        t203 = [vbslq_u32(t205[0], t206[0], t203[0]), vbslq_u32(t205[1], t206[1], t203[1])];
    }
    res = t203;
    return res;
}

#[inline(always)]
unsafe fn b12_r0lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t207: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x8[0])), vmovl_u32(vget_high_u32(x8[0])), vmovl_u32(vget_low_u32(x8[1])), vmovl_u32(vget_high_u32(x8[1]))];
    let t208: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x16[0])), vmovl_u32(vget_high_u32(x16[0])), vmovl_u32(vget_low_u32(x16[1])), vmovl_u32(vget_high_u32(x16[1]))];
    let t209: [uint64x2_t; 4] = [vaddq_u64(t207[0], t208[0]), vaddq_u64(t207[1], t208[1]), vaddq_u64(t207[2], t208[2]), vaddq_u64(t207[3], t208[3])];
    let t210 = b16_r0ls_p(l, lid0, live, arg0, t209, x24);
    x32 = t210;
    let t211: [uint64x2_t; 4];
    if ((x24) >= (8u32)) {
        t211 = x32;
    } else {
        let t212 = b17_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t213 = [vdupq_n_u64(t212); 4];
        let t214: [uint64x2_t; 4] = [vandq_u64(x32[0], t213[0]), vandq_u64(x32[1], t213[1]), vandq_u64(x32[2], t213[2]), vandq_u64(x32[3], t213[3])];
        t211 = t214;
    }
    res = t211;
    return res;
}

#[inline(always)]
unsafe fn b13_r0r1sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2], x32: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t215 = b18_r0sls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t215;
    return res;
}

#[inline(always)]
unsafe fn b14_sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t216 = [vdupq_n_u32(x0); 2];
    let t217: [uint32x4_t; 2] = [vcltq_u32(t216[0], x8[0]), vcltq_u32(t216[1], x8[1])];
    let mut t218: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t219: [uint32x4_t; 2] = [vandq_u32(live[0], t217[0]), vandq_u32(live[1], t217[1])];
    let t220: [uint32x4_t; 2] = [vbicq_u32(live[0], t217[0]), vbicq_u32(live[1], t217[1])];
    {
        let t221 = [vdupq_n_u32(x0); 2];
        t218 = [vbslq_u32(t219[0], t221[0], t218[0]), vbslq_u32(t219[1], t221[1], t218[1])];
    }
    {
        t218 = [vbslq_u32(t220[0], x8[0], t218[0]), vbslq_u32(t220[1], x8[1], t218[1])];
    }
    res = t218;
    return res;
}

#[inline(always)]
unsafe fn b15_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t222: [uint32x4_t; 2] = [vcltq_u32(x0[0], x8[0]), vcltq_u32(x0[1], x8[1])];
    let mut t223: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t224: [uint32x4_t; 2] = [vandq_u32(live[0], t222[0]), vandq_u32(live[1], t222[1])];
    let t225: [uint32x4_t; 2] = [vbicq_u32(live[0], t222[0]), vbicq_u32(live[1], t222[1])];
    {
        t223 = [vbslq_u32(t224[0], x0[0], t223[0]), vbslq_u32(t224[1], x0[1], t223[1])];
    }
    {
        t223 = [vbslq_u32(t225[0], x8[0], t223[0]), vbslq_u32(t225[1], x8[1], t223[1])];
    }
    res = t223;
    return res;
}

#[inline(always)]
unsafe fn b16_r0ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint64x2_t; 4], x16: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t226 = [vdupq_n_u64(!(7u64)); 4];
    let t227: [uint64x2_t; 4] = [vandq_u64(x8[0], t226[0]), vandq_u64(x8[1], t226[1]), vandq_u64(x8[2], t226[2]), vandq_u64(x8[3], t226[3])];
    let t228: [uint64x2_t; 4] = [vandq_u64(t227[0], vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0]))))), vandq_u64(t227[1], vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0]))))), vandq_u64(t227[2], vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1]))))), vandq_u64(t227[3], vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))))];
    let t229: [uint64x2_t; 4] = [vaddq_u64(vdupq_n_u64((arg0) as u64), t228[0]), vaddq_u64(vdupq_n_u64((arg0) as u64), t228[1]), vaddq_u64(vdupq_n_u64((arg0) as u64), t228[2]), vaddq_u64(vdupq_n_u64((arg0) as u64), t228[3])];
    x24 = t229;
    let t230 = [vdupq_n_u64(7u64); 4];
    let t231: [uint64x2_t; 4] = [vandq_u64(x8[0], t230[0]), vandq_u64(x8[1], t230[1]), vandq_u64(x8[2], t230[2]), vandq_u64(x8[3], t230[3])];
    let t232: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t231[0]), vmovn_u64(t231[1])), vcombine_u32(vmovn_u64(t231[2]), vmovn_u64(t231[3]))];
    let t233 = [vdupq_n_u32(8u32); 2];
    let t234: [uint32x4_t; 2] = [vmulq_u32(t232[0], t233[0]), vmulq_u32(t232[1], t233[1])];
    x32 = t234;
    let mut t235: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    t235[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[0]) as usize as *const u64).read_unaligned(), t235[0]);
    t235[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[0]) as usize as *const u64).read_unaligned(), t235[0]);
    t235[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[1]) as usize as *const u64).read_unaligned(), t235[1]);
    t235[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[1]) as usize as *const u64).read_unaligned(), t235[1]);
    t235[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[2]) as usize as *const u64).read_unaligned(), t235[2]);
    t235[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[2]) as usize as *const u64).read_unaligned(), t235[2]);
    t235[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[3]) as usize as *const u64).read_unaligned(), t235[3]);
    t235[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[3]) as usize as *const u64).read_unaligned(), t235[3]);
    let t236: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x32[0])), vmovl_u32(vget_high_u32(x32[0])), vmovl_u32(vget_low_u32(x32[1])), vmovl_u32(vget_high_u32(x32[1]))];
    let t237: [uint64x2_t; 4] = [vshlq_u64(t235[0], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t236[0], vdupq_n_u64(63))))), vshlq_u64(t235[1], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t236[1], vdupq_n_u64(63))))), vshlq_u64(t235[2], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t236[2], vdupq_n_u64(63))))), vshlq_u64(t235[3], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t236[3], vdupq_n_u64(63)))))];
    x40 = t237;
    let t238 = [vdupq_n_u32(0u32); 2];
    let t239: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t238[0])), vmvnq_u32(vceqq_u32(x32[1], t238[1]))];
    let t240 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t241: [uint32x4_t; 2] = [vaddq_u32(x32[0], t240[0]), vaddq_u32(x32[1], t240[1])];
    let t242 = [vdupq_n_u32(64u32); 2];
    let t243: [uint32x4_t; 2] = [vcgtq_u32(t241[0], t242[0]), vcgtq_u32(t241[1], t242[1])];
    let t244: [uint32x4_t; 2] = [vandq_u32(t239[0], t243[0]), vandq_u32(t239[1], t243[1])];
    let mut t245: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t246: [uint32x4_t; 2] = [vandq_u32(live[0], t244[0]), vandq_u32(live[1], t244[1])];
    let t247: [uint32x4_t; 2] = [vbicq_u32(live[0], t244[0]), vbicq_u32(live[1], t244[1])];
    if any(&t246) {
        let t248 = [vdupq_n_u64((1u64).wrapping_mul(8u64)); 4];
        let t249: [uint64x2_t; 4] = [vaddq_u64(x24[0], t248[0]), vaddq_u64(x24[1], t248[1]), vaddq_u64(x24[2], t248[2]), vaddq_u64(x24[3], t248[3])];
        let mut t250: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
        t250[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t249[0]) as usize as *const u64).read_unaligned(), t250[0]);
        t250[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t249[0]) as usize as *const u64).read_unaligned(), t250[0]);
        t250[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t249[1]) as usize as *const u64).read_unaligned(), t250[1]);
        t250[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t249[1]) as usize as *const u64).read_unaligned(), t250[1]);
        t250[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t249[2]) as usize as *const u64).read_unaligned(), t250[2]);
        t250[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t249[2]) as usize as *const u64).read_unaligned(), t250[2]);
        t250[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t249[3]) as usize as *const u64).read_unaligned(), t250[3]);
        t250[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t249[3]) as usize as *const u64).read_unaligned(), t250[3]);
        let t251: [uint32x4_t; 2] = [vsubq_u32(t242[0], x32[0]), vsubq_u32(t242[1], x32[1])];
        let t252: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t251[0])), vmovl_u32(vget_high_u32(t251[0])), vmovl_u32(vget_low_u32(t251[1])), vmovl_u32(vget_high_u32(t251[1]))];
        let t253: [uint64x2_t; 4] = [vshlq_u64(t250[0], vreinterpretq_s64_u64(vandq_u64(t252[0], vdupq_n_u64(63)))), vshlq_u64(t250[1], vreinterpretq_s64_u64(vandq_u64(t252[1], vdupq_n_u64(63)))), vshlq_u64(t250[2], vreinterpretq_s64_u64(vandq_u64(t252[2], vdupq_n_u64(63)))), vshlq_u64(t250[3], vreinterpretq_s64_u64(vandq_u64(t252[3], vdupq_n_u64(63))))];
        let t254: [uint64x2_t; 4] = [vorrq_u64(x40[0], t253[0]), vorrq_u64(x40[1], t253[1]), vorrq_u64(x40[2], t253[2]), vorrq_u64(x40[3], t253[3])];
        t245 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t246[0])))), t254[0], t245[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t246[0])))), t254[1], t245[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t246[1])))), t254[2], t245[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t246[1])))), t254[3], t245[3])];
    }
    {
        t245 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t247[0])))), x40[0], t245[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t247[0])))), x40[1], t245[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t247[1])))), x40[2], t245[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t247[1])))), x40[3], t245[3])];
    }
    res = t245;
    return res;
}

#[inline(always)]
unsafe fn b17_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t255: u64;
    if ((x0) >= (64u32)) {
        t255 = !(0u64);
    } else {
        t255 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t255;
    return res;
}

#[inline(always)]
unsafe fn b18_r0sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16: [uint32x4_t; 2], x24: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t256 = [vdupq_n_u64(((x8) as u64)); 4];
    let t257: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x16[0])), vmovl_u32(vget_high_u32(x16[0])), vmovl_u32(vget_low_u32(x16[1])), vmovl_u32(vget_high_u32(x16[1]))];
    let t258: [uint64x2_t; 4] = [vaddq_u64(t256[0], t257[0]), vaddq_u64(t256[1], t257[1]), vaddq_u64(t256[2], t257[2]), vaddq_u64(t256[3], t257[3])];
    let t259 = b16_r0ls_p(l, lid0, live, arg0, t258, x24);
    x32 = t259;
    let t260: [uint64x2_t; 4];
    if ((x24) >= (8u32)) {
        t260 = x32;
    } else {
        let t261 = b17_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t262 = [vdupq_n_u64(t261); 4];
        let t263: [uint64x2_t; 4] = [vandq_u64(x32[0], t262[0]), vandq_u64(x32[1], t262[1]), vandq_u64(x32[2], t262[2]), vandq_u64(x32[3], t262[3])];
        t260 = t263;
    }
    res = t260;
    return res;
}

/// The group `l.group`'s threads, 8 at a time: `argv[j]` the address of argument `j`'s bytes (a buffer's address,
/// a value's bytes).
///
/// # Safety
///
/// `argv` holds the kernel's arguments, its buffers as large as it reads and writes; `l` is a 1-D group's launch.
pub unsafe fn parent_all_w8(argv: *const *const u8, l: &Launch) {
    let n = l.block[0] * l.block[1] * l.block[2];
    let arg0: *const u8 = (*argv.add(0) as *const *const u8).read_unaligned();
    let arg1: *const u8 = (*argv.add(1) as *const *const u8).read_unaligned();
    let arg2: *const u8 = (*argv.add(2) as *const *const u8).read_unaligned();
    let x24: *const u8 = *argv.add(3);
    let arg4: *const u8 = (*argv.add(4) as *const *const u8).read_unaligned();
    let x48: u32 = (*argv.add(5) as *const u32).read_unaligned();
    let mut t264: bool = !(arg4).is_null();
    if t264 {
        let mut t265: bool = ((l.group[0]) >= ((arg4.wrapping_add((((x48) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t265 {
            t265 = ((l.group[1]) >= ((arg4.wrapping_add(((((x48).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t266: bool = t265;
        if !t266 {
            t266 = ((l.group[2]) >= ((arg4.wrapping_add(((((x48).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t264 = t266;
    }
    if t264 {
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
        let t267: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t267;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1r2sg_p(l, lid0, live, arg0, arg1, arg2, x24);
        }
    }
}
