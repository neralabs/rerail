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
        let mut t21 = live;
        loop {
            // Lanes part: each its own rounds.
            let t23 = [vdupq_n_u32(7u32); 2];
            let t24: [uint32x4_t; 2] = [vcltq_u32(x48[0], t23[0]), vcltq_u32(x48[1], t23[1])];
            let t25 = [vdupq_n_u32(4294967295u32); 2];
            let t26: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x40[0], t25[0])), vmvnq_u32(vceqq_u32(x40[1], t25[1]))];
            let t27: [uint32x4_t; 2] = [vandq_u32(t24[0], t26[0]), vandq_u32(t24[1], t26[1])];
            let t28 = [vdupq_n_u32(2147483648u32); 2];
            let t29: [uint32x4_t; 2] = [vandq_u32(x40[0], t28[0]), vandq_u32(x40[1], t28[1])];
            let t30: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t29[0], t11[0])), vmvnq_u32(vceqq_u32(t29[1], t11[1]))];
            let t31: [uint32x4_t; 2] = [vandq_u32(t27[0], t30[0]), vandq_u32(t27[1], t30[1])];
            t21 = [vandq_u32(t21[0], t31[0]), vandq_u32(t21[1], t31[1])];
            if !(any(&t21)) { break; }
            let t33 = [vdupq_n_u32(2147483647u32); 2];
            let t34: [uint32x4_t; 2] = [vandq_u32(x40[0], t33[0]), vandq_u32(x40[1], t33[1])];
            let t35 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(t21[0], t34[0]), vandq_u32(t21[1], t34[1]))));
            let t36: [uint32x4_t; 2] = [vbslq_u32(t21[0], t34[0], t35), vbslq_u32(t21[1], t34[1], t35)];
            let t37 = arg0 as *const u32;
            let mut t38: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t38[0] = vld1q_lane_u32::<0>(t37.add(vgetq_lane_u32::<0>(t36[0]) as usize), t38[0]);
            t38[0] = vld1q_lane_u32::<1>(t37.add(vgetq_lane_u32::<1>(t36[0]) as usize), t38[0]);
            t38[0] = vld1q_lane_u32::<2>(t37.add(vgetq_lane_u32::<2>(t36[0]) as usize), t38[0]);
            t38[0] = vld1q_lane_u32::<3>(t37.add(vgetq_lane_u32::<3>(t36[0]) as usize), t38[0]);
            t38[1] = vld1q_lane_u32::<0>(t37.add(vgetq_lane_u32::<0>(t36[1]) as usize), t38[1]);
            t38[1] = vld1q_lane_u32::<1>(t37.add(vgetq_lane_u32::<1>(t36[1]) as usize), t38[1]);
            t38[1] = vld1q_lane_u32::<2>(t37.add(vgetq_lane_u32::<2>(t36[1]) as usize), t38[1]);
            t38[1] = vld1q_lane_u32::<3>(t37.add(vgetq_lane_u32::<3>(t36[1]) as usize), t38[1]);
            x40 = [vbslq_u32(t21[0], t38[0], x40[0]), vbslq_u32(t21[1], t38[1], x40[1])];
            let t39: [uint32x4_t; 2] = [vandq_u32(t21[0], live[0]), vandq_u32(t21[1], live[1])];
            let t40 = x48;
            x48 = [vsubq_u32(x48[0], t39[0]), vsubq_u32(x48[1], t39[1])];
        }
        let t41 = [vdupq_n_u32(4294967295u32); 2];
        let t42: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x40[0], t41[0])), vmvnq_u32(vceqq_u32(x40[1], t41[1]))];
        let t43 = [vdupq_n_u32(2147483648u32); 2];
        let t44: [uint32x4_t; 2] = [vandq_u32(x40[0], t43[0]), vandq_u32(x40[1], t43[1])];
        let t45: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t44[0], t11[0])), vmvnq_u32(vceqq_u32(t44[1], t11[1]))];
        let t46: [uint32x4_t; 2] = [vandq_u32(t42[0], t45[0]), vandq_u32(t42[1], t45[1])];
        let t47: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t46[0]), vshrq_n_u32::<31>(t46[1])];
        x48 = t47;
        let t49: [uint32x4_t; 2] = [vtstq_u32(x48[0], x48[0]), vtstq_u32(x48[1], x48[1])];
        let mut t48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t50: [uint32x4_t; 2] = [vandq_u32(live[0], t49[0]), vandq_u32(live[1], t49[1])];
        let t51: [uint32x4_t; 2] = [vbicq_u32(live[0], t49[0]), vbicq_u32(live[1], t49[1])];
        {
            t48 = [vbslq_u32(t50[0], t41[0], t48[0]), vbslq_u32(t50[1], t41[1], t48[1])];
        }
        {
            t48 = [vbslq_u32(t51[0], x40[0], t48[0]), vbslq_u32(t51[1], x40[1], t48[1])];
        }
        let t52 = (arg1 as *mut u32).add((gid0) as usize);
        if all(&live) {
            vst1q_u32(t52.add(0), t48[0]);
            vst1q_u32(t52.add(4), t48[1]);
        } else {
            let mut t53w = 0u32;
            let t53 = core::hint::black_box(&raw mut t53w);
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t52.add(0), t53).write(vgetq_lane_u32::<0>(t48[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t52.add(1), t53).write(vgetq_lane_u32::<1>(t48[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t52.add(2), t53).write(vgetq_lane_u32::<2>(t48[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t52.add(3), t53).write(vgetq_lane_u32::<3>(t48[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t52.add(4), t53).write(vgetq_lane_u32::<0>(t48[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t52.add(5), t53).write(vgetq_lane_u32::<1>(t48[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t52.add(6), t53).write(vgetq_lane_u32::<2>(t48[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t52.add(7), t53).write(vgetq_lane_u32::<3>(t48[1]));
        }
        let t54: [uint32x4_t; 2] = [vtstq_u32(x48[0], x48[0]), vtstq_u32(x48[1], x48[1])];
        let t55: [uint32x4_t; 2] = [vandq_u32(live[0], t54[0]), vandq_u32(live[1], t54[1])];
        {
            let t56: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[1].as_ptr()))];
            if vgetq_lane_u32::<0>(t55[0]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<0>(t56[0])); }
            if vgetq_lane_u32::<1>(t55[0]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<1>(t56[0])); }
            if vgetq_lane_u32::<2>(t55[0]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<2>(t56[0])); }
            if vgetq_lane_u32::<3>(t55[0]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<3>(t56[0])); }
            if vgetq_lane_u32::<0>(t55[1]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<0>(t56[1])); }
            if vgetq_lane_u32::<1>(t55[1]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<1>(t56[1])); }
            if vgetq_lane_u32::<2>(t55[1]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<2>(t56[1])); }
            if vgetq_lane_u32::<3>(t55[1]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<3>(t56[1])); }
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
        let t57 = (arg0 as *const u32).add((gid0) as usize);
        let t58: [uint32x4_t; 2] = [vld1q_u32(t57.add(0)), vld1q_u32(t57.add(4))];
        x40 = t58;
        let t59 = [vdupq_n_u32(0u32); 2];
        x48 = t59;
        let mut t78 = live;
        loop {
            // Lanes part: each its own rounds.
            let t80 = [vdupq_n_u32(7u32); 2];
            let t81: [uint32x4_t; 2] = [vcltq_u32(x48[0], t80[0]), vcltq_u32(x48[1], t80[1])];
            let t82 = [vdupq_n_u32(4294967295u32); 2];
            let t83: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x40[0], t82[0])), vmvnq_u32(vceqq_u32(x40[1], t82[1]))];
            let t84: [uint32x4_t; 2] = [vandq_u32(t81[0], t83[0]), vandq_u32(t81[1], t83[1])];
            let t85 = [vdupq_n_u32(2147483648u32); 2];
            let t86: [uint32x4_t; 2] = [vandq_u32(x40[0], t85[0]), vandq_u32(x40[1], t85[1])];
            let t87: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t86[0], t59[0])), vmvnq_u32(vceqq_u32(t86[1], t59[1]))];
            let t88: [uint32x4_t; 2] = [vandq_u32(t84[0], t87[0]), vandq_u32(t84[1], t87[1])];
            t78 = [vandq_u32(t78[0], t88[0]), vandq_u32(t78[1], t88[1])];
            if !(any(&t78)) { break; }
            let t90 = [vdupq_n_u32(2147483647u32); 2];
            let t91: [uint32x4_t; 2] = [vandq_u32(x40[0], t90[0]), vandq_u32(x40[1], t90[1])];
            let t92: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t93: [uint32x4_t; 2] = [vbslq_u32(t78[0], t91[0], t92[0]), vbslq_u32(t78[1], t91[1], t92[1])];
            let t94 = arg0 as *const u32;
            let mut t95: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t95[0] = vld1q_lane_u32::<0>(t94.add(vgetq_lane_u32::<0>(t93[0]) as usize), t95[0]);
            t95[0] = vld1q_lane_u32::<1>(t94.add(vgetq_lane_u32::<1>(t93[0]) as usize), t95[0]);
            t95[0] = vld1q_lane_u32::<2>(t94.add(vgetq_lane_u32::<2>(t93[0]) as usize), t95[0]);
            t95[0] = vld1q_lane_u32::<3>(t94.add(vgetq_lane_u32::<3>(t93[0]) as usize), t95[0]);
            t95[1] = vld1q_lane_u32::<0>(t94.add(vgetq_lane_u32::<0>(t93[1]) as usize), t95[1]);
            t95[1] = vld1q_lane_u32::<1>(t94.add(vgetq_lane_u32::<1>(t93[1]) as usize), t95[1]);
            t95[1] = vld1q_lane_u32::<2>(t94.add(vgetq_lane_u32::<2>(t93[1]) as usize), t95[1]);
            t95[1] = vld1q_lane_u32::<3>(t94.add(vgetq_lane_u32::<3>(t93[1]) as usize), t95[1]);
            x40 = [vbslq_u32(t78[0], t95[0], x40[0]), vbslq_u32(t78[1], t95[1], x40[1])];
            let t96: [uint32x4_t; 2] = [vandq_u32(t78[0], live[0]), vandq_u32(t78[1], live[1])];
            let t97 = x48;
            x48 = [vsubq_u32(x48[0], t96[0]), vsubq_u32(x48[1], t96[1])];
        }
        let t98 = [vdupq_n_u32(4294967295u32); 2];
        let t99: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x40[0], t98[0])), vmvnq_u32(vceqq_u32(x40[1], t98[1]))];
        let t100 = [vdupq_n_u32(2147483648u32); 2];
        let t101: [uint32x4_t; 2] = [vandq_u32(x40[0], t100[0]), vandq_u32(x40[1], t100[1])];
        let t102: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t101[0], t59[0])), vmvnq_u32(vceqq_u32(t101[1], t59[1]))];
        let t103: [uint32x4_t; 2] = [vandq_u32(t99[0], t102[0]), vandq_u32(t99[1], t102[1])];
        let t104: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t103[0]), vshrq_n_u32::<31>(t103[1])];
        x48 = t104;
        let t106: [uint32x4_t; 2] = [vtstq_u32(x48[0], x48[0]), vtstq_u32(x48[1], x48[1])];
        let mut t105: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t107: [uint32x4_t; 2] = [vmvnq_u32(t106[0]), vmvnq_u32(t106[1])];
        {
            t105 = [vbslq_u32(t106[0], t98[0], t105[0]), vbslq_u32(t106[1], t98[1], t105[1])];
        }
        {
            t105 = [vbslq_u32(t107[0], x40[0], t105[0]), vbslq_u32(t107[1], x40[1], t105[1])];
        }
        let t108 = (arg1 as *mut u32).add((gid0) as usize);
        vst1q_u32(t108.add(0), t105[0]);
        vst1q_u32(t108.add(4), t105[1]);
        let t109: [uint32x4_t; 2] = [vtstq_u32(x48[0], x48[0]), vtstq_u32(x48[1], x48[1])];
        {
            let t110: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[1].as_ptr()))];
            if vgetq_lane_u32::<0>(t109[0]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<0>(t110[0])); }
            if vgetq_lane_u32::<1>(t109[0]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<1>(t110[0])); }
            if vgetq_lane_u32::<2>(t109[0]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<2>(t110[0])); }
            if vgetq_lane_u32::<3>(t109[0]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<3>(t110[0])); }
            if vgetq_lane_u32::<0>(t109[1]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<0>(t110[1])); }
            if vgetq_lane_u32::<1>(t109[1]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<1>(t110[1])); }
            if vgetq_lane_u32::<2>(t109[1]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<2>(t110[1])); }
            if vgetq_lane_u32::<3>(t109[1]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<3>(t110[1])); }
        }
    } else if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= (((x16 as *const u32).read_unaligned()) as u64)) {
        return;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t111: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t112 = [vdupq_n_u32((x16 as *const u32).read_unaligned()); 2];
        let t113: [uint32x4_t; 2] = [vcgeq_u32(t111[0], t112[0]), vcgeq_u32(t111[1], t112[1])];
        let t114: [uint32x4_t; 2] = [vandq_u32(live[0], t113[0]), vandq_u32(live[1], t113[1])];
        {
            live = [vbicq_u32(live[0], t114[0]), vbicq_u32(live[1], t114[1])];
        }
        if any(&live) {
            let mut t115: [uint32x4_t; 2];
            if all(&live) {
                let t116 = (arg0 as *const u32).add((gid0) as usize);
                t115 = [vld1q_u32(t116.add(0)), vld1q_u32(t116.add(4))];
            } else {
                let t117 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t111[0]), vandq_u32(live[1], t111[1]))));
                let t118: [uint32x4_t; 2] = [vbslq_u32(live[0], t111[0], t117), vbslq_u32(live[1], t111[1], t117)];
                let t119 = arg0 as *const u32;
                let mut t120: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                t120[0] = vld1q_lane_u32::<0>(t119.add(vgetq_lane_u32::<0>(t118[0]) as usize), t120[0]);
                t120[0] = vld1q_lane_u32::<1>(t119.add(vgetq_lane_u32::<1>(t118[0]) as usize), t120[0]);
                t120[0] = vld1q_lane_u32::<2>(t119.add(vgetq_lane_u32::<2>(t118[0]) as usize), t120[0]);
                t120[0] = vld1q_lane_u32::<3>(t119.add(vgetq_lane_u32::<3>(t118[0]) as usize), t120[0]);
                t120[1] = vld1q_lane_u32::<0>(t119.add(vgetq_lane_u32::<0>(t118[1]) as usize), t120[1]);
                t120[1] = vld1q_lane_u32::<1>(t119.add(vgetq_lane_u32::<1>(t118[1]) as usize), t120[1]);
                t120[1] = vld1q_lane_u32::<2>(t119.add(vgetq_lane_u32::<2>(t118[1]) as usize), t120[1]);
                t120[1] = vld1q_lane_u32::<3>(t119.add(vgetq_lane_u32::<3>(t118[1]) as usize), t120[1]);
                t115 = t120;
            }
            x40 = t115;
            let t121 = [vdupq_n_u32(0u32); 2];
            x48 = t121;
            let mut t131 = live;
            loop {
                // Lanes part: each its own rounds.
                let t133 = [vdupq_n_u32(7u32); 2];
                let t134: [uint32x4_t; 2] = [vcltq_u32(x48[0], t133[0]), vcltq_u32(x48[1], t133[1])];
                let t135 = [vdupq_n_u32(4294967295u32); 2];
                let t136: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x40[0], t135[0])), vmvnq_u32(vceqq_u32(x40[1], t135[1]))];
                let t137: [uint32x4_t; 2] = [vandq_u32(t134[0], t136[0]), vandq_u32(t134[1], t136[1])];
                let t138 = [vdupq_n_u32(2147483648u32); 2];
                let t139: [uint32x4_t; 2] = [vandq_u32(x40[0], t138[0]), vandq_u32(x40[1], t138[1])];
                let t140: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t139[0], t121[0])), vmvnq_u32(vceqq_u32(t139[1], t121[1]))];
                let t141: [uint32x4_t; 2] = [vandq_u32(t137[0], t140[0]), vandq_u32(t137[1], t140[1])];
                t131 = [vandq_u32(t131[0], t141[0]), vandq_u32(t131[1], t141[1])];
                if !(any(&t131)) { break; }
                let t143 = [vdupq_n_u32(2147483647u32); 2];
                let t144: [uint32x4_t; 2] = [vandq_u32(x40[0], t143[0]), vandq_u32(x40[1], t143[1])];
                let t145 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(t131[0], t144[0]), vandq_u32(t131[1], t144[1]))));
                let t146: [uint32x4_t; 2] = [vbslq_u32(t131[0], t144[0], t145), vbslq_u32(t131[1], t144[1], t145)];
                let t147 = arg0 as *const u32;
                let mut t148: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                t148[0] = vld1q_lane_u32::<0>(t147.add(vgetq_lane_u32::<0>(t146[0]) as usize), t148[0]);
                t148[0] = vld1q_lane_u32::<1>(t147.add(vgetq_lane_u32::<1>(t146[0]) as usize), t148[0]);
                t148[0] = vld1q_lane_u32::<2>(t147.add(vgetq_lane_u32::<2>(t146[0]) as usize), t148[0]);
                t148[0] = vld1q_lane_u32::<3>(t147.add(vgetq_lane_u32::<3>(t146[0]) as usize), t148[0]);
                t148[1] = vld1q_lane_u32::<0>(t147.add(vgetq_lane_u32::<0>(t146[1]) as usize), t148[1]);
                t148[1] = vld1q_lane_u32::<1>(t147.add(vgetq_lane_u32::<1>(t146[1]) as usize), t148[1]);
                t148[1] = vld1q_lane_u32::<2>(t147.add(vgetq_lane_u32::<2>(t146[1]) as usize), t148[1]);
                t148[1] = vld1q_lane_u32::<3>(t147.add(vgetq_lane_u32::<3>(t146[1]) as usize), t148[1]);
                x40 = [vbslq_u32(t131[0], t148[0], x40[0]), vbslq_u32(t131[1], t148[1], x40[1])];
                let t149: [uint32x4_t; 2] = [vandq_u32(t131[0], live[0]), vandq_u32(t131[1], live[1])];
                let t150 = x48;
                x48 = [vsubq_u32(x48[0], t149[0]), vsubq_u32(x48[1], t149[1])];
            }
            let t151 = [vdupq_n_u32(4294967295u32); 2];
            let t152: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x40[0], t151[0])), vmvnq_u32(vceqq_u32(x40[1], t151[1]))];
            let t153 = [vdupq_n_u32(2147483648u32); 2];
            let t154: [uint32x4_t; 2] = [vandq_u32(x40[0], t153[0]), vandq_u32(x40[1], t153[1])];
            let t155: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t154[0], t121[0])), vmvnq_u32(vceqq_u32(t154[1], t121[1]))];
            let t156: [uint32x4_t; 2] = [vandq_u32(t152[0], t155[0]), vandq_u32(t152[1], t155[1])];
            let t157: [uint32x4_t; 2] = [vshrq_n_u32::<31>(t156[0]), vshrq_n_u32::<31>(t156[1])];
            x48 = t157;
            let t159: [uint32x4_t; 2] = [vtstq_u32(x48[0], x48[0]), vtstq_u32(x48[1], x48[1])];
            let mut t158: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t160: [uint32x4_t; 2] = [vandq_u32(live[0], t159[0]), vandq_u32(live[1], t159[1])];
            let t161: [uint32x4_t; 2] = [vbicq_u32(live[0], t159[0]), vbicq_u32(live[1], t159[1])];
            {
                t158 = [vbslq_u32(t160[0], t151[0], t158[0]), vbslq_u32(t160[1], t151[1], t158[1])];
            }
            {
                t158 = [vbslq_u32(t161[0], x40[0], t158[0]), vbslq_u32(t161[1], x40[1], t158[1])];
            }
            let t162 = (arg1 as *mut u32).add((gid0) as usize);
            if all(&live) {
                vst1q_u32(t162.add(0), t158[0]);
                vst1q_u32(t162.add(4), t158[1]);
            } else {
                let mut t163w = 0u32;
                let t163 = core::hint::black_box(&raw mut t163w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t162.add(0), t163).write(vgetq_lane_u32::<0>(t158[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t162.add(1), t163).write(vgetq_lane_u32::<1>(t158[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t162.add(2), t163).write(vgetq_lane_u32::<2>(t158[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t162.add(3), t163).write(vgetq_lane_u32::<3>(t158[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t162.add(4), t163).write(vgetq_lane_u32::<0>(t158[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t162.add(5), t163).write(vgetq_lane_u32::<1>(t158[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t162.add(6), t163).write(vgetq_lane_u32::<2>(t158[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t162.add(7), t163).write(vgetq_lane_u32::<3>(t158[1]));
            }
            let t164: [uint32x4_t; 2] = [vtstq_u32(x48[0], x48[0]), vtstq_u32(x48[1], x48[1])];
            let t165: [uint32x4_t; 2] = [vandq_u32(live[0], t164[0]), vandq_u32(live[1], t164[1])];
            {
                let t166: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[1].as_ptr()))];
                if vgetq_lane_u32::<0>(t165[0]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<0>(t166[0])); }
                if vgetq_lane_u32::<1>(t165[0]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<1>(t166[0])); }
                if vgetq_lane_u32::<2>(t165[0]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<2>(t166[0])); }
                if vgetq_lane_u32::<3>(t165[0]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<3>(t166[0])); }
                if vgetq_lane_u32::<0>(t165[1]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<0>(t166[1])); }
                if vgetq_lane_u32::<1>(t165[1]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<1>(t166[1])); }
                if vgetq_lane_u32::<2>(t165[1]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<2>(t166[1])); }
                if vgetq_lane_u32::<3>(t165[1]) != 0 { (arg1.wrapping_add(((((x16 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<3>(t166[1])); }
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
    let mut t167: bool = !(arg3).is_null();
    if t167 {
        let mut t168: bool = ((l.group[0]) >= ((arg3.wrapping_add((((x40) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t168 {
            t168 = ((l.group[1]) >= ((arg3.wrapping_add(((((x40).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t169: bool = t168;
        if !t169 {
            t169 = ((l.group[2]) >= ((arg3.wrapping_add(((((x40).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t167 = t169;
    }
    if t167 {
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
        let t170: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t170;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1sg_p(l, lid0, live, arg0, arg1, x16);
        }
    }
}
