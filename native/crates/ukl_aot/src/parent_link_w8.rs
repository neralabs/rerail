//! `parent_link` at 8 lanes (2 NEON registers), written by rerail-ukl-vec's Rust (`rs.rs`) from its vector tape: do
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
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let t1: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t2 = [vdupq_n_u32((x16 as *const u32).read_unaligned()); 2];
    let t3: [uint32x4_t; 2] = [vcgeq_u32(t1[0], t2[0]), vcgeq_u32(t1[1], t2[1])];
    let t4: [uint32x4_t; 2] = [vandq_u32(live[0], t3[0]), vandq_u32(live[1], t3[1])];
    {
        live = [vbicq_u32(live[0], t4[0]), vbicq_u32(live[1], t4[1])];
    }
    if any(&live) {
        let mut t5: [uint32x4_t; 2];
        if all(&live) {
            let t6 = (arg0 as *const u32).add((gid0) as usize);
            t5 = [vld1q_u32(t6.add(0)), vld1q_u32(t6.add(4))];
        } else {
            let t7 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1[0]), vandq_u32(live[1], t1[1]))));
            let t8: [uint32x4_t; 2] = [vbslq_u32(live[0], t1[0], t7), vbslq_u32(live[1], t1[1], t7)];
            let t9 = arg0 as *const u32;
            let mut t10: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t10[0] = vld1q_lane_u32::<0>(t9.add(vgetq_lane_u32::<0>(t8[0]) as usize), t10[0]);
            t10[0] = vld1q_lane_u32::<1>(t9.add(vgetq_lane_u32::<1>(t8[0]) as usize), t10[0]);
            t10[0] = vld1q_lane_u32::<2>(t9.add(vgetq_lane_u32::<2>(t8[0]) as usize), t10[0]);
            t10[0] = vld1q_lane_u32::<3>(t9.add(vgetq_lane_u32::<3>(t8[0]) as usize), t10[0]);
            t10[1] = vld1q_lane_u32::<0>(t9.add(vgetq_lane_u32::<0>(t8[1]) as usize), t10[1]);
            t10[1] = vld1q_lane_u32::<1>(t9.add(vgetq_lane_u32::<1>(t8[1]) as usize), t10[1]);
            t10[1] = vld1q_lane_u32::<2>(t9.add(vgetq_lane_u32::<2>(t8[1]) as usize), t10[1]);
            t10[1] = vld1q_lane_u32::<3>(t9.add(vgetq_lane_u32::<3>(t8[1]) as usize), t10[1]);
            t5 = t10;
        }
        x40 = t5;
        let t11 = [vdupq_n_u32(0u32); 2];
        x48 = t11;
        let mut t12 = live;
        loop {
            // Lanes part: each its own rounds.
            let t14 = [vdupq_n_u32(7u32); 2];
            let t15: [uint32x4_t; 2] = [vcltq_u32(x48[0], t14[0]), vcltq_u32(x48[1], t14[1])];
            let t16 = [vdupq_n_u32(4294967295u32); 2];
            let t17: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x40[0], t16[0])), vmvnq_u32(vceqq_u32(x40[1], t16[1]))];
            let t18: [uint32x4_t; 2] = [vandq_u32(t15[0], t17[0]), vandq_u32(t15[1], t17[1])];
            let t19 = [vdupq_n_u32(2147483648u32); 2];
            let t20: [uint32x4_t; 2] = [vandq_u32(x40[0], t19[0]), vandq_u32(x40[1], t19[1])];
            let t21: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t20[0], t11[0])), vmvnq_u32(vceqq_u32(t20[1], t11[1]))];
            let t22: [uint32x4_t; 2] = [vandq_u32(t18[0], t21[0]), vandq_u32(t18[1], t21[1])];
            t12 = [vandq_u32(t12[0], t22[0]), vandq_u32(t12[1], t22[1])];
            if !(any(&t12)) { break; }
            let t24 = [vdupq_n_u32(2147483647u32); 2];
            let t25: [uint32x4_t; 2] = [vandq_u32(x40[0], t24[0]), vandq_u32(x40[1], t24[1])];
            let t26 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(t12[0], t25[0]), vandq_u32(t12[1], t25[1]))));
            let t27: [uint32x4_t; 2] = [vbslq_u32(t12[0], t25[0], t26), vbslq_u32(t12[1], t25[1], t26)];
            let t28 = arg0 as *const u32;
            let mut t29: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t29[0] = vld1q_lane_u32::<0>(t28.add(vgetq_lane_u32::<0>(t27[0]) as usize), t29[0]);
            t29[0] = vld1q_lane_u32::<1>(t28.add(vgetq_lane_u32::<1>(t27[0]) as usize), t29[0]);
            t29[0] = vld1q_lane_u32::<2>(t28.add(vgetq_lane_u32::<2>(t27[0]) as usize), t29[0]);
            t29[0] = vld1q_lane_u32::<3>(t28.add(vgetq_lane_u32::<3>(t27[0]) as usize), t29[0]);
            t29[1] = vld1q_lane_u32::<0>(t28.add(vgetq_lane_u32::<0>(t27[1]) as usize), t29[1]);
            t29[1] = vld1q_lane_u32::<1>(t28.add(vgetq_lane_u32::<1>(t27[1]) as usize), t29[1]);
            t29[1] = vld1q_lane_u32::<2>(t28.add(vgetq_lane_u32::<2>(t27[1]) as usize), t29[1]);
            t29[1] = vld1q_lane_u32::<3>(t28.add(vgetq_lane_u32::<3>(t27[1]) as usize), t29[1]);
            x40 = [vbslq_u32(t12[0], t29[0], x40[0]), vbslq_u32(t12[1], t29[1], x40[1])];
            let t30: [uint32x4_t; 2] = [vandq_u32(t12[0], live[0]), vandq_u32(t12[1], live[1])];
            let t31 = x48;
            x48 = [vsubq_u32(x48[0], t30[0]), vsubq_u32(x48[1], t30[1])];
        }
        let t32 = [vdupq_n_u32(4294967295u32); 2];
        let t33: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x40[0], t32[0])), vmvnq_u32(vceqq_u32(x40[1], t32[1]))];
        let t34 = [vdupq_n_u32(2147483648u32); 2];
        let t35: [uint32x4_t; 2] = [vandq_u32(x40[0], t34[0]), vandq_u32(x40[1], t34[1])];
        let t36: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t35[0], t11[0])), vmvnq_u32(vceqq_u32(t35[1], t11[1]))];
        let t37: [uint32x4_t; 2] = [vandq_u32(t33[0], t36[0]), vandq_u32(t33[1], t36[1])];
        let t38: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t37[0]), vshrq_n_u32::<31>(t37[1])];
        x48 = t38;
        let t40: [uint32x4_t; 2] = [vtstq_u32(x48[0], x48[0]), vtstq_u32(x48[1], x48[1])];
        let mut t39: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t41: [uint32x4_t; 2] = [vandq_u32(live[0], t40[0]), vandq_u32(live[1], t40[1])];
        let t42: [uint32x4_t; 2] = [vbicq_u32(live[0], t40[0]), vbicq_u32(live[1], t40[1])];
        {
            t39 = [vbslq_u32(t41[0], t32[0], t39[0]), vbslq_u32(t41[1], t32[1], t39[1])];
        }
        {
            t39 = [vbslq_u32(t42[0], x40[0], t39[0]), vbslq_u32(t42[1], x40[1], t39[1])];
        }
        let t43 = (arg1 as *mut u32).add((gid0) as usize);
        if all(&live) {
            vst1q_u32(t43.add(0), t39[0]);
            vst1q_u32(t43.add(4), t39[1]);
        } else {
            let mut t44w = 0u32;
            let t44 = core::hint::black_box(&raw mut t44w);
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t43.add(0), t44).write(vgetq_lane_u32::<0>(t39[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t43.add(1), t44).write(vgetq_lane_u32::<1>(t39[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t43.add(2), t44).write(vgetq_lane_u32::<2>(t39[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t43.add(3), t44).write(vgetq_lane_u32::<3>(t39[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t43.add(4), t44).write(vgetq_lane_u32::<0>(t39[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t43.add(5), t44).write(vgetq_lane_u32::<1>(t39[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t43.add(6), t44).write(vgetq_lane_u32::<2>(t39[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t43.add(7), t44).write(vgetq_lane_u32::<3>(t39[1]));
        }
        let t45: [uint32x4_t; 2] = [vtstq_u32(x48[0], x48[0]), vtstq_u32(x48[1], x48[1])];
        let t46: [uint32x4_t; 2] = [vandq_u32(live[0], t45[0]), vandq_u32(live[1], t45[1])];
        {
            let t47: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[1].as_ptr()))];
            if vgetq_lane_u32::<0>(t46[0]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<0>(t47[0])); }
            if vgetq_lane_u32::<1>(t46[0]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<1>(t47[0])); }
            if vgetq_lane_u32::<2>(t46[0]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<2>(t47[0])); }
            if vgetq_lane_u32::<3>(t46[0]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<3>(t47[0])); }
            if vgetq_lane_u32::<0>(t46[1]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<0>(t47[1])); }
            if vgetq_lane_u32::<1>(t46[1]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<1>(t47[1])); }
            if vgetq_lane_u32::<2>(t46[1]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<2>(t47[1])); }
            if vgetq_lane_u32::<3>(t46[1]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<3>(t47[1])); }
        }
    }
}

#[inline(always)]
unsafe fn b0_r0r1sg_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if ((gid0) as u64 + 7) < (((x16 as *const u32).read_unaligned()) as u64) {
        let t48 = (arg0 as *const u32).add((gid0) as usize);
        let t49: [uint32x4_t; 2] = [vld1q_u32(t48.add(0)), vld1q_u32(t48.add(4))];
        x40 = t49;
        let t50 = [vdupq_n_u32(0u32); 2];
        x48 = t50;
        let mut t60 = live;
        loop {
            // Lanes part: each its own rounds.
            let t62 = [vdupq_n_u32(7u32); 2];
            let t63: [uint32x4_t; 2] = [vcltq_u32(x48[0], t62[0]), vcltq_u32(x48[1], t62[1])];
            let t64 = [vdupq_n_u32(4294967295u32); 2];
            let t65: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x40[0], t64[0])), vmvnq_u32(vceqq_u32(x40[1], t64[1]))];
            let t66: [uint32x4_t; 2] = [vandq_u32(t63[0], t65[0]), vandq_u32(t63[1], t65[1])];
            let t67 = [vdupq_n_u32(2147483648u32); 2];
            let t68: [uint32x4_t; 2] = [vandq_u32(x40[0], t67[0]), vandq_u32(x40[1], t67[1])];
            let t69: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t68[0], t50[0])), vmvnq_u32(vceqq_u32(t68[1], t50[1]))];
            let t70: [uint32x4_t; 2] = [vandq_u32(t66[0], t69[0]), vandq_u32(t66[1], t69[1])];
            t60 = [vandq_u32(t60[0], t70[0]), vandq_u32(t60[1], t70[1])];
            if !(any(&t60)) { break; }
            let t72 = [vdupq_n_u32(2147483647u32); 2];
            let t73: [uint32x4_t; 2] = [vandq_u32(x40[0], t72[0]), vandq_u32(x40[1], t72[1])];
            let t74: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t75: [uint32x4_t; 2] = [vbslq_u32(t60[0], t73[0], t74[0]), vbslq_u32(t60[1], t73[1], t74[1])];
            let t76 = arg0 as *const u32;
            let mut t77: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t77[0] = vld1q_lane_u32::<0>(t76.add(vgetq_lane_u32::<0>(t75[0]) as usize), t77[0]);
            t77[0] = vld1q_lane_u32::<1>(t76.add(vgetq_lane_u32::<1>(t75[0]) as usize), t77[0]);
            t77[0] = vld1q_lane_u32::<2>(t76.add(vgetq_lane_u32::<2>(t75[0]) as usize), t77[0]);
            t77[0] = vld1q_lane_u32::<3>(t76.add(vgetq_lane_u32::<3>(t75[0]) as usize), t77[0]);
            t77[1] = vld1q_lane_u32::<0>(t76.add(vgetq_lane_u32::<0>(t75[1]) as usize), t77[1]);
            t77[1] = vld1q_lane_u32::<1>(t76.add(vgetq_lane_u32::<1>(t75[1]) as usize), t77[1]);
            t77[1] = vld1q_lane_u32::<2>(t76.add(vgetq_lane_u32::<2>(t75[1]) as usize), t77[1]);
            t77[1] = vld1q_lane_u32::<3>(t76.add(vgetq_lane_u32::<3>(t75[1]) as usize), t77[1]);
            x40 = [vbslq_u32(t60[0], t77[0], x40[0]), vbslq_u32(t60[1], t77[1], x40[1])];
            let t78: [uint32x4_t; 2] = [vandq_u32(t60[0], live[0]), vandq_u32(t60[1], live[1])];
            let t79 = x48;
            x48 = [vsubq_u32(x48[0], t78[0]), vsubq_u32(x48[1], t78[1])];
        }
        let t80 = [vdupq_n_u32(4294967295u32); 2];
        let t81: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x40[0], t80[0])), vmvnq_u32(vceqq_u32(x40[1], t80[1]))];
        let t82 = [vdupq_n_u32(2147483648u32); 2];
        let t83: [uint32x4_t; 2] = [vandq_u32(x40[0], t82[0]), vandq_u32(x40[1], t82[1])];
        let t84: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t83[0], t50[0])), vmvnq_u32(vceqq_u32(t83[1], t50[1]))];
        let t85: [uint32x4_t; 2] = [vandq_u32(t81[0], t84[0]), vandq_u32(t81[1], t84[1])];
        let t86: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t85[0]), vshrq_n_u32::<31>(t85[1])];
        x48 = t86;
        let t88: [uint32x4_t; 2] = [vtstq_u32(x48[0], x48[0]), vtstq_u32(x48[1], x48[1])];
        let mut t87: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t89: [uint32x4_t; 2] = [vmvnq_u32(t88[0]), vmvnq_u32(t88[1])];
        {
            t87 = [vbslq_u32(t88[0], t80[0], t87[0]), vbslq_u32(t88[1], t80[1], t87[1])];
        }
        {
            t87 = [vbslq_u32(t89[0], x40[0], t87[0]), vbslq_u32(t89[1], x40[1], t87[1])];
        }
        let t90 = (arg1 as *mut u32).add((gid0) as usize);
        vst1q_u32(t90.add(0), t87[0]);
        vst1q_u32(t90.add(4), t87[1]);
        let t91: [uint32x4_t; 2] = [vtstq_u32(x48[0], x48[0]), vtstq_u32(x48[1], x48[1])];
        {
            let t92: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[1].as_ptr()))];
            if vgetq_lane_u32::<0>(t91[0]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<0>(t92[0])); }
            if vgetq_lane_u32::<1>(t91[0]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<1>(t92[0])); }
            if vgetq_lane_u32::<2>(t91[0]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<2>(t92[0])); }
            if vgetq_lane_u32::<3>(t91[0]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<3>(t92[0])); }
            if vgetq_lane_u32::<0>(t91[1]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<0>(t92[1])); }
            if vgetq_lane_u32::<1>(t91[1]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<1>(t92[1])); }
            if vgetq_lane_u32::<2>(t91[1]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<2>(t92[1])); }
            if vgetq_lane_u32::<3>(t91[1]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<3>(t92[1])); }
        }
    } else if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= (((x16 as *const u32).read_unaligned()) as u64)) {
        return;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t93: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t94 = [vdupq_n_u32((x16 as *const u32).read_unaligned()); 2];
        let t95: [uint32x4_t; 2] = [vcgeq_u32(t93[0], t94[0]), vcgeq_u32(t93[1], t94[1])];
        let t96: [uint32x4_t; 2] = [vandq_u32(live[0], t95[0]), vandq_u32(live[1], t95[1])];
        {
            live = [vbicq_u32(live[0], t96[0]), vbicq_u32(live[1], t96[1])];
        }
        if any(&live) {
            let mut t97: [uint32x4_t; 2];
            if all(&live) {
                let t98 = (arg0 as *const u32).add((gid0) as usize);
                t97 = [vld1q_u32(t98.add(0)), vld1q_u32(t98.add(4))];
            } else {
                let t99 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t93[0]), vandq_u32(live[1], t93[1]))));
                let t100: [uint32x4_t; 2] = [vbslq_u32(live[0], t93[0], t99), vbslq_u32(live[1], t93[1], t99)];
                let t101 = arg0 as *const u32;
                let mut t102: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                t102[0] = vld1q_lane_u32::<0>(t101.add(vgetq_lane_u32::<0>(t100[0]) as usize), t102[0]);
                t102[0] = vld1q_lane_u32::<1>(t101.add(vgetq_lane_u32::<1>(t100[0]) as usize), t102[0]);
                t102[0] = vld1q_lane_u32::<2>(t101.add(vgetq_lane_u32::<2>(t100[0]) as usize), t102[0]);
                t102[0] = vld1q_lane_u32::<3>(t101.add(vgetq_lane_u32::<3>(t100[0]) as usize), t102[0]);
                t102[1] = vld1q_lane_u32::<0>(t101.add(vgetq_lane_u32::<0>(t100[1]) as usize), t102[1]);
                t102[1] = vld1q_lane_u32::<1>(t101.add(vgetq_lane_u32::<1>(t100[1]) as usize), t102[1]);
                t102[1] = vld1q_lane_u32::<2>(t101.add(vgetq_lane_u32::<2>(t100[1]) as usize), t102[1]);
                t102[1] = vld1q_lane_u32::<3>(t101.add(vgetq_lane_u32::<3>(t100[1]) as usize), t102[1]);
                t97 = t102;
            }
            x40 = t97;
            let t103 = [vdupq_n_u32(0u32); 2];
            x48 = t103;
            let mut t104 = live;
            loop {
                // Lanes part: each its own rounds.
                let t106 = [vdupq_n_u32(7u32); 2];
                let t107: [uint32x4_t; 2] = [vcltq_u32(x48[0], t106[0]), vcltq_u32(x48[1], t106[1])];
                let t108 = [vdupq_n_u32(4294967295u32); 2];
                let t109: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x40[0], t108[0])), vmvnq_u32(vceqq_u32(x40[1], t108[1]))];
                let t110: [uint32x4_t; 2] = [vandq_u32(t107[0], t109[0]), vandq_u32(t107[1], t109[1])];
                let t111 = [vdupq_n_u32(2147483648u32); 2];
                let t112: [uint32x4_t; 2] = [vandq_u32(x40[0], t111[0]), vandq_u32(x40[1], t111[1])];
                let t113: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t112[0], t103[0])), vmvnq_u32(vceqq_u32(t112[1], t103[1]))];
                let t114: [uint32x4_t; 2] = [vandq_u32(t110[0], t113[0]), vandq_u32(t110[1], t113[1])];
                t104 = [vandq_u32(t104[0], t114[0]), vandq_u32(t104[1], t114[1])];
                if !(any(&t104)) { break; }
                let t116 = [vdupq_n_u32(2147483647u32); 2];
                let t117: [uint32x4_t; 2] = [vandq_u32(x40[0], t116[0]), vandq_u32(x40[1], t116[1])];
                let t118 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(t104[0], t117[0]), vandq_u32(t104[1], t117[1]))));
                let t119: [uint32x4_t; 2] = [vbslq_u32(t104[0], t117[0], t118), vbslq_u32(t104[1], t117[1], t118)];
                let t120 = arg0 as *const u32;
                let mut t121: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                t121[0] = vld1q_lane_u32::<0>(t120.add(vgetq_lane_u32::<0>(t119[0]) as usize), t121[0]);
                t121[0] = vld1q_lane_u32::<1>(t120.add(vgetq_lane_u32::<1>(t119[0]) as usize), t121[0]);
                t121[0] = vld1q_lane_u32::<2>(t120.add(vgetq_lane_u32::<2>(t119[0]) as usize), t121[0]);
                t121[0] = vld1q_lane_u32::<3>(t120.add(vgetq_lane_u32::<3>(t119[0]) as usize), t121[0]);
                t121[1] = vld1q_lane_u32::<0>(t120.add(vgetq_lane_u32::<0>(t119[1]) as usize), t121[1]);
                t121[1] = vld1q_lane_u32::<1>(t120.add(vgetq_lane_u32::<1>(t119[1]) as usize), t121[1]);
                t121[1] = vld1q_lane_u32::<2>(t120.add(vgetq_lane_u32::<2>(t119[1]) as usize), t121[1]);
                t121[1] = vld1q_lane_u32::<3>(t120.add(vgetq_lane_u32::<3>(t119[1]) as usize), t121[1]);
                x40 = [vbslq_u32(t104[0], t121[0], x40[0]), vbslq_u32(t104[1], t121[1], x40[1])];
                let t122: [uint32x4_t; 2] = [vandq_u32(t104[0], live[0]), vandq_u32(t104[1], live[1])];
                let t123 = x48;
                x48 = [vsubq_u32(x48[0], t122[0]), vsubq_u32(x48[1], t122[1])];
            }
            let t124 = [vdupq_n_u32(4294967295u32); 2];
            let t125: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x40[0], t124[0])), vmvnq_u32(vceqq_u32(x40[1], t124[1]))];
            let t126 = [vdupq_n_u32(2147483648u32); 2];
            let t127: [uint32x4_t; 2] = [vandq_u32(x40[0], t126[0]), vandq_u32(x40[1], t126[1])];
            let t128: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t127[0], t103[0])), vmvnq_u32(vceqq_u32(t127[1], t103[1]))];
            let t129: [uint32x4_t; 2] = [vandq_u32(t125[0], t128[0]), vandq_u32(t125[1], t128[1])];
            let t130: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t129[0]), vshrq_n_u32::<31>(t129[1])];
            x48 = t130;
            let t132: [uint32x4_t; 2] = [vtstq_u32(x48[0], x48[0]), vtstq_u32(x48[1], x48[1])];
            let mut t131: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t133: [uint32x4_t; 2] = [vandq_u32(live[0], t132[0]), vandq_u32(live[1], t132[1])];
            let t134: [uint32x4_t; 2] = [vbicq_u32(live[0], t132[0]), vbicq_u32(live[1], t132[1])];
            {
                t131 = [vbslq_u32(t133[0], t124[0], t131[0]), vbslq_u32(t133[1], t124[1], t131[1])];
            }
            {
                t131 = [vbslq_u32(t134[0], x40[0], t131[0]), vbslq_u32(t134[1], x40[1], t131[1])];
            }
            let t135 = (arg1 as *mut u32).add((gid0) as usize);
            if all(&live) {
                vst1q_u32(t135.add(0), t131[0]);
                vst1q_u32(t135.add(4), t131[1]);
            } else {
                let mut t136w = 0u32;
                let t136 = core::hint::black_box(&raw mut t136w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t135.add(0), t136).write(vgetq_lane_u32::<0>(t131[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t135.add(1), t136).write(vgetq_lane_u32::<1>(t131[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t135.add(2), t136).write(vgetq_lane_u32::<2>(t131[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t135.add(3), t136).write(vgetq_lane_u32::<3>(t131[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t135.add(4), t136).write(vgetq_lane_u32::<0>(t131[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t135.add(5), t136).write(vgetq_lane_u32::<1>(t131[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t135.add(6), t136).write(vgetq_lane_u32::<2>(t131[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t135.add(7), t136).write(vgetq_lane_u32::<3>(t131[1]));
            }
            let t137: [uint32x4_t; 2] = [vtstq_u32(x48[0], x48[0]), vtstq_u32(x48[1], x48[1])];
            let t138: [uint32x4_t; 2] = [vandq_u32(live[0], t137[0]), vandq_u32(live[1], t137[1])];
            {
                let t139: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[1].as_ptr()))];
                if vgetq_lane_u32::<0>(t138[0]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<0>(t139[0])); }
                if vgetq_lane_u32::<1>(t138[0]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<1>(t139[0])); }
                if vgetq_lane_u32::<2>(t138[0]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<2>(t139[0])); }
                if vgetq_lane_u32::<3>(t138[0]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<3>(t139[0])); }
                if vgetq_lane_u32::<0>(t138[1]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<0>(t139[1])); }
                if vgetq_lane_u32::<1>(t138[1]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<1>(t139[1])); }
                if vgetq_lane_u32::<2>(t138[1]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<2>(t139[1])); }
                if vgetq_lane_u32::<3>(t138[1]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<3>(t139[1])); }
            }
        }
    }
}

/// The group `l.group`'s threads, 8 at a time: `argv[j]` the address of argument `j`'s bytes (a buffer's address,
/// a value's bytes).
///
/// # Safety
///
/// `argv` holds the kernel's arguments, its buffers as large as it reads and writes; `l` is a 1-D group's launch.
pub unsafe fn parent_link_w8(argv: *const *const u8, l: &Launch) {
    let n = l.block[0] * l.block[1] * l.block[2];
    let arg0: *const u8 = (*argv.add(0) as *const *const u8).read_unaligned();
    let arg1: *const u8 = (*argv.add(1) as *const *const u8).read_unaligned();
    let x16: *const u8 = *argv.add(2);
    let arg3: *const u8 = (*argv.add(3) as *const *const u8).read_unaligned();
    let x40: u32 = (*argv.add(4) as *const u32).read_unaligned();
    let mut t140: bool = !(arg3).is_null();
    if t140 {
        let mut t141: bool = ((l.group[0]) >= ((arg3.wrapping_add((((x40) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t141 {
            t141 = ((l.group[1]) >= ((arg3.wrapping_add(((((x40).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t142: bool = t141;
        if !t142 {
            t142 = ((l.group[2]) >= ((arg3.wrapping_add(((((x40).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t140 = t142;
    }
    if t140 {
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
        let t143: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t143;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1sg_p(l, lid0, live, arg0, arg1, x16);
        }
    }
}
