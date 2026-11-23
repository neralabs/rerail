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
unsafe fn b0_r0r1sg_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x2: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: u32 = 0;
    let mut x6: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let t1: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t2 = [vdupq_n_u32((x2 as *const u32).read_unaligned()); 2];
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
        x4 = t5;
        x5 = 0u32;
        let mut t20 = live;
        loop {
            // Lanes part: each its own rounds.
            let t23 = [vdupq_n_u32(4294967295u32); 2];
            let t24: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t23[0])), vmvnq_u32(vceqq_u32(x4[1], t23[1]))];
            let t25: [uint32x4_t; 2] = [vandq_u32(vdupq_n_u32(if ((x5) < (7u32)) { u32::MAX } else { 0 }), t24[0]), vandq_u32(vdupq_n_u32(if ((x5) < (7u32)) { u32::MAX } else { 0 }), t24[1])];
            let t26 = [vdupq_n_u32(2147483648u32); 2];
            let t27: [uint32x4_t; 2] = [vandq_u32(x4[0], t26[0]), vandq_u32(x4[1], t26[1])];
            let t28 = [vdupq_n_u32(0u32); 2];
            let t29: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t27[0], t28[0])), vmvnq_u32(vceqq_u32(t27[1], t28[1]))];
            let t30: [uint32x4_t; 2] = [vandq_u32(t25[0], t29[0]), vandq_u32(t25[1], t29[1])];
            t20 = [vandq_u32(t20[0], t30[0]), vandq_u32(t20[1], t30[1])];
            if !(any(&t20)) { break; }
            let t32 = [vdupq_n_u32(2147483647u32); 2];
            let t33: [uint32x4_t; 2] = [vandq_u32(x4[0], t32[0]), vandq_u32(x4[1], t32[1])];
            let t34 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(t20[0], t33[0]), vandq_u32(t20[1], t33[1]))));
            let t35: [uint32x4_t; 2] = [vbslq_u32(t20[0], t33[0], t34), vbslq_u32(t20[1], t33[1], t34)];
            let t36 = arg0 as *const u32;
            let mut t37: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t37[0] = vld1q_lane_u32::<0>(t36.add(vgetq_lane_u32::<0>(t35[0]) as usize), t37[0]);
            t37[0] = vld1q_lane_u32::<1>(t36.add(vgetq_lane_u32::<1>(t35[0]) as usize), t37[0]);
            t37[0] = vld1q_lane_u32::<2>(t36.add(vgetq_lane_u32::<2>(t35[0]) as usize), t37[0]);
            t37[0] = vld1q_lane_u32::<3>(t36.add(vgetq_lane_u32::<3>(t35[0]) as usize), t37[0]);
            t37[1] = vld1q_lane_u32::<0>(t36.add(vgetq_lane_u32::<0>(t35[1]) as usize), t37[1]);
            t37[1] = vld1q_lane_u32::<1>(t36.add(vgetq_lane_u32::<1>(t35[1]) as usize), t37[1]);
            t37[1] = vld1q_lane_u32::<2>(t36.add(vgetq_lane_u32::<2>(t35[1]) as usize), t37[1]);
            t37[1] = vld1q_lane_u32::<3>(t36.add(vgetq_lane_u32::<3>(t35[1]) as usize), t37[1]);
            x4 = [vbslq_u32(t20[0], t37[0], x4[0]), vbslq_u32(t20[1], t37[1], x4[1])];
            let t38: [uint32x4_t; 2] = [vandq_u32(t20[0], live[0]), vandq_u32(t20[1], live[1])];
            let t39 = x5;
            x5 = (x5).wrapping_add(1u32);
        }
        let t40 = [vdupq_n_u32(4294967295u32); 2];
        let t41: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t40[0])), vmvnq_u32(vceqq_u32(x4[1], t40[1]))];
        let t42 = [vdupq_n_u32(2147483648u32); 2];
        let t43: [uint32x4_t; 2] = [vandq_u32(x4[0], t42[0]), vandq_u32(x4[1], t42[1])];
        let t44 = [vdupq_n_u32(0u32); 2];
        let t45: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t43[0], t44[0])), vmvnq_u32(vceqq_u32(t43[1], t44[1]))];
        let t46: [uint32x4_t; 2] = [vandq_u32(t41[0], t45[0]), vandq_u32(t41[1], t45[1])];
        x6 = t46;
        let mut t47: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t48: [uint32x4_t; 2] = [vandq_u32(live[0], x6[0]), vandq_u32(live[1], x6[1])];
        let t49: [uint32x4_t; 2] = [vbicq_u32(live[0], x6[0]), vbicq_u32(live[1], x6[1])];
        {
            t47 = [vbslq_u32(t48[0], t40[0], t47[0]), vbslq_u32(t48[1], t40[1], t47[1])];
        }
        {
            t47 = [vbslq_u32(t49[0], x4[0], t47[0]), vbslq_u32(t49[1], x4[1], t47[1])];
        }
        let t50 = (arg1 as *mut u32).add((gid0) as usize);
        if all(&live) {
            vst1q_u32(t50.add(0), t47[0]);
            vst1q_u32(t50.add(4), t47[1]);
        } else {
            let mut t51w = 0u32;
            let t51 = core::hint::black_box(&raw mut t51w);
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t50.add(0), t51).write(vgetq_lane_u32::<0>(t47[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t50.add(1), t51).write(vgetq_lane_u32::<1>(t47[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t50.add(2), t51).write(vgetq_lane_u32::<2>(t47[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t50.add(3), t51).write(vgetq_lane_u32::<3>(t47[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t50.add(4), t51).write(vgetq_lane_u32::<0>(t47[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t50.add(5), t51).write(vgetq_lane_u32::<1>(t47[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t50.add(6), t51).write(vgetq_lane_u32::<2>(t47[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t50.add(7), t51).write(vgetq_lane_u32::<3>(t47[1]));
        }
        let t52: [uint32x4_t; 2] = [vandq_u32(live[0], x6[0]), vandq_u32(live[1], x6[1])];
        {
            let t53: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[1].as_ptr()))];
            if vgetq_lane_u32::<0>(t52[0]) != 0 { (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<0>(t53[0])); }
            if vgetq_lane_u32::<1>(t52[0]) != 0 { (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<1>(t53[0])); }
            if vgetq_lane_u32::<2>(t52[0]) != 0 { (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<2>(t53[0])); }
            if vgetq_lane_u32::<3>(t52[0]) != 0 { (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<3>(t53[0])); }
            if vgetq_lane_u32::<0>(t52[1]) != 0 { (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<0>(t53[1])); }
            if vgetq_lane_u32::<1>(t52[1]) != 0 { (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<1>(t53[1])); }
            if vgetq_lane_u32::<2>(t52[1]) != 0 { (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<2>(t53[1])); }
            if vgetq_lane_u32::<3>(t52[1]) != 0 { (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<3>(t53[1])); }
        }
    }
}

#[inline(always)]
unsafe fn b0_r0r1sg_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x2: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x4: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x5: u32 = 0;
    let mut x6: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if ((gid0) as u64 + 7) < (((x2 as *const u32).read_unaligned()) as u64) {
        let t54 = (arg0 as *const u32).add((gid0) as usize);
        let t55: [uint32x4_t; 2] = [vld1q_u32(t54.add(0)), vld1q_u32(t54.add(4))];
        x4 = t55;
        x5 = 0u32;
        let mut t74 = live;
        loop {
            // Lanes part: each its own rounds.
            let t77 = [vdupq_n_u32(4294967295u32); 2];
            let t78: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t77[0])), vmvnq_u32(vceqq_u32(x4[1], t77[1]))];
            let t79: [uint32x4_t; 2] = [vandq_u32(vdupq_n_u32(if ((x5) < (7u32)) { u32::MAX } else { 0 }), t78[0]), vandq_u32(vdupq_n_u32(if ((x5) < (7u32)) { u32::MAX } else { 0 }), t78[1])];
            let t80 = [vdupq_n_u32(2147483648u32); 2];
            let t81: [uint32x4_t; 2] = [vandq_u32(x4[0], t80[0]), vandq_u32(x4[1], t80[1])];
            let t82 = [vdupq_n_u32(0u32); 2];
            let t83: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t81[0], t82[0])), vmvnq_u32(vceqq_u32(t81[1], t82[1]))];
            let t84: [uint32x4_t; 2] = [vandq_u32(t79[0], t83[0]), vandq_u32(t79[1], t83[1])];
            t74 = [vandq_u32(t74[0], t84[0]), vandq_u32(t74[1], t84[1])];
            if !(any(&t74)) { break; }
            let t86 = [vdupq_n_u32(2147483647u32); 2];
            let t87: [uint32x4_t; 2] = [vandq_u32(x4[0], t86[0]), vandq_u32(x4[1], t86[1])];
            let t88: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t89: [uint32x4_t; 2] = [vbslq_u32(t74[0], t87[0], t88[0]), vbslq_u32(t74[1], t87[1], t88[1])];
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
            x4 = [vbslq_u32(t74[0], t91[0], x4[0]), vbslq_u32(t74[1], t91[1], x4[1])];
            let t92: [uint32x4_t; 2] = [vandq_u32(t74[0], live[0]), vandq_u32(t74[1], live[1])];
            let t93 = x5;
            x5 = (x5).wrapping_add(1u32);
        }
        let t94 = [vdupq_n_u32(4294967295u32); 2];
        let t95: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t94[0])), vmvnq_u32(vceqq_u32(x4[1], t94[1]))];
        let t96 = [vdupq_n_u32(2147483648u32); 2];
        let t97: [uint32x4_t; 2] = [vandq_u32(x4[0], t96[0]), vandq_u32(x4[1], t96[1])];
        let t98 = [vdupq_n_u32(0u32); 2];
        let t99: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t97[0], t98[0])), vmvnq_u32(vceqq_u32(t97[1], t98[1]))];
        let t100: [uint32x4_t; 2] = [vandq_u32(t95[0], t99[0]), vandq_u32(t95[1], t99[1])];
        x6 = t100;
        let mut t101: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t102: [uint32x4_t; 2] = [vmvnq_u32(x6[0]), vmvnq_u32(x6[1])];
        {
            t101 = [vbslq_u32(x6[0], t94[0], t101[0]), vbslq_u32(x6[1], t94[1], t101[1])];
        }
        {
            t101 = [vbslq_u32(t102[0], x4[0], t101[0]), vbslq_u32(t102[1], x4[1], t101[1])];
        }
        let t103 = (arg1 as *mut u32).add((gid0) as usize);
        vst1q_u32(t103.add(0), t101[0]);
        vst1q_u32(t103.add(4), t101[1]);
        {
            let t104: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[1].as_ptr()))];
            if vgetq_lane_u32::<0>(x6[0]) != 0 { (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<0>(t104[0])); }
            if vgetq_lane_u32::<1>(x6[0]) != 0 { (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<1>(t104[0])); }
            if vgetq_lane_u32::<2>(x6[0]) != 0 { (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<2>(t104[0])); }
            if vgetq_lane_u32::<3>(x6[0]) != 0 { (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<3>(t104[0])); }
            if vgetq_lane_u32::<0>(x6[1]) != 0 { (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<0>(t104[1])); }
            if vgetq_lane_u32::<1>(x6[1]) != 0 { (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<1>(t104[1])); }
            if vgetq_lane_u32::<2>(x6[1]) != 0 { (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<2>(t104[1])); }
            if vgetq_lane_u32::<3>(x6[1]) != 0 { (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<3>(t104[1])); }
        }
    } else if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= (((x2 as *const u32).read_unaligned()) as u64)) {
        return;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t105: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t106 = [vdupq_n_u32((x2 as *const u32).read_unaligned()); 2];
        let t107: [uint32x4_t; 2] = [vcgeq_u32(t105[0], t106[0]), vcgeq_u32(t105[1], t106[1])];
        let t108: [uint32x4_t; 2] = [vandq_u32(live[0], t107[0]), vandq_u32(live[1], t107[1])];
        {
            live = [vbicq_u32(live[0], t108[0]), vbicq_u32(live[1], t108[1])];
        }
        if any(&live) {
            let mut t109: [uint32x4_t; 2];
            if all(&live) {
                let t110 = (arg0 as *const u32).add((gid0) as usize);
                t109 = [vld1q_u32(t110.add(0)), vld1q_u32(t110.add(4))];
            } else {
                let t111 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t105[0]), vandq_u32(live[1], t105[1]))));
                let t112: [uint32x4_t; 2] = [vbslq_u32(live[0], t105[0], t111), vbslq_u32(live[1], t105[1], t111)];
                let t113 = arg0 as *const u32;
                let mut t114: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                t114[0] = vld1q_lane_u32::<0>(t113.add(vgetq_lane_u32::<0>(t112[0]) as usize), t114[0]);
                t114[0] = vld1q_lane_u32::<1>(t113.add(vgetq_lane_u32::<1>(t112[0]) as usize), t114[0]);
                t114[0] = vld1q_lane_u32::<2>(t113.add(vgetq_lane_u32::<2>(t112[0]) as usize), t114[0]);
                t114[0] = vld1q_lane_u32::<3>(t113.add(vgetq_lane_u32::<3>(t112[0]) as usize), t114[0]);
                t114[1] = vld1q_lane_u32::<0>(t113.add(vgetq_lane_u32::<0>(t112[1]) as usize), t114[1]);
                t114[1] = vld1q_lane_u32::<1>(t113.add(vgetq_lane_u32::<1>(t112[1]) as usize), t114[1]);
                t114[1] = vld1q_lane_u32::<2>(t113.add(vgetq_lane_u32::<2>(t112[1]) as usize), t114[1]);
                t114[1] = vld1q_lane_u32::<3>(t113.add(vgetq_lane_u32::<3>(t112[1]) as usize), t114[1]);
                t109 = t114;
            }
            x4 = t109;
            x5 = 0u32;
            let mut t124 = live;
            loop {
                // Lanes part: each its own rounds.
                let t127 = [vdupq_n_u32(4294967295u32); 2];
                let t128: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t127[0])), vmvnq_u32(vceqq_u32(x4[1], t127[1]))];
                let t129: [uint32x4_t; 2] = [vandq_u32(vdupq_n_u32(if ((x5) < (7u32)) { u32::MAX } else { 0 }), t128[0]), vandq_u32(vdupq_n_u32(if ((x5) < (7u32)) { u32::MAX } else { 0 }), t128[1])];
                let t130 = [vdupq_n_u32(2147483648u32); 2];
                let t131: [uint32x4_t; 2] = [vandq_u32(x4[0], t130[0]), vandq_u32(x4[1], t130[1])];
                let t132 = [vdupq_n_u32(0u32); 2];
                let t133: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t131[0], t132[0])), vmvnq_u32(vceqq_u32(t131[1], t132[1]))];
                let t134: [uint32x4_t; 2] = [vandq_u32(t129[0], t133[0]), vandq_u32(t129[1], t133[1])];
                t124 = [vandq_u32(t124[0], t134[0]), vandq_u32(t124[1], t134[1])];
                if !(any(&t124)) { break; }
                let t136 = [vdupq_n_u32(2147483647u32); 2];
                let t137: [uint32x4_t; 2] = [vandq_u32(x4[0], t136[0]), vandq_u32(x4[1], t136[1])];
                let t138 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(t124[0], t137[0]), vandq_u32(t124[1], t137[1]))));
                let t139: [uint32x4_t; 2] = [vbslq_u32(t124[0], t137[0], t138), vbslq_u32(t124[1], t137[1], t138)];
                let t140 = arg0 as *const u32;
                let mut t141: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                t141[0] = vld1q_lane_u32::<0>(t140.add(vgetq_lane_u32::<0>(t139[0]) as usize), t141[0]);
                t141[0] = vld1q_lane_u32::<1>(t140.add(vgetq_lane_u32::<1>(t139[0]) as usize), t141[0]);
                t141[0] = vld1q_lane_u32::<2>(t140.add(vgetq_lane_u32::<2>(t139[0]) as usize), t141[0]);
                t141[0] = vld1q_lane_u32::<3>(t140.add(vgetq_lane_u32::<3>(t139[0]) as usize), t141[0]);
                t141[1] = vld1q_lane_u32::<0>(t140.add(vgetq_lane_u32::<0>(t139[1]) as usize), t141[1]);
                t141[1] = vld1q_lane_u32::<1>(t140.add(vgetq_lane_u32::<1>(t139[1]) as usize), t141[1]);
                t141[1] = vld1q_lane_u32::<2>(t140.add(vgetq_lane_u32::<2>(t139[1]) as usize), t141[1]);
                t141[1] = vld1q_lane_u32::<3>(t140.add(vgetq_lane_u32::<3>(t139[1]) as usize), t141[1]);
                x4 = [vbslq_u32(t124[0], t141[0], x4[0]), vbslq_u32(t124[1], t141[1], x4[1])];
                let t142: [uint32x4_t; 2] = [vandq_u32(t124[0], live[0]), vandq_u32(t124[1], live[1])];
                let t143 = x5;
                x5 = (x5).wrapping_add(1u32);
            }
            let t144 = [vdupq_n_u32(4294967295u32); 2];
            let t145: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t144[0])), vmvnq_u32(vceqq_u32(x4[1], t144[1]))];
            let t146 = [vdupq_n_u32(2147483648u32); 2];
            let t147: [uint32x4_t; 2] = [vandq_u32(x4[0], t146[0]), vandq_u32(x4[1], t146[1])];
            let t148 = [vdupq_n_u32(0u32); 2];
            let t149: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t147[0], t148[0])), vmvnq_u32(vceqq_u32(t147[1], t148[1]))];
            let t150: [uint32x4_t; 2] = [vandq_u32(t145[0], t149[0]), vandq_u32(t145[1], t149[1])];
            x6 = t150;
            let mut t151: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t152: [uint32x4_t; 2] = [vandq_u32(live[0], x6[0]), vandq_u32(live[1], x6[1])];
            let t153: [uint32x4_t; 2] = [vbicq_u32(live[0], x6[0]), vbicq_u32(live[1], x6[1])];
            {
                t151 = [vbslq_u32(t152[0], t144[0], t151[0]), vbslq_u32(t152[1], t144[1], t151[1])];
            }
            {
                t151 = [vbslq_u32(t153[0], x4[0], t151[0]), vbslq_u32(t153[1], x4[1], t151[1])];
            }
            let t154 = (arg1 as *mut u32).add((gid0) as usize);
            if all(&live) {
                vst1q_u32(t154.add(0), t151[0]);
                vst1q_u32(t154.add(4), t151[1]);
            } else {
                let mut t155w = 0u32;
                let t155 = core::hint::black_box(&raw mut t155w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t154.add(0), t155).write(vgetq_lane_u32::<0>(t151[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t154.add(1), t155).write(vgetq_lane_u32::<1>(t151[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t154.add(2), t155).write(vgetq_lane_u32::<2>(t151[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t154.add(3), t155).write(vgetq_lane_u32::<3>(t151[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t154.add(4), t155).write(vgetq_lane_u32::<0>(t151[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t154.add(5), t155).write(vgetq_lane_u32::<1>(t151[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t154.add(6), t155).write(vgetq_lane_u32::<2>(t151[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t154.add(7), t155).write(vgetq_lane_u32::<3>(t151[1]));
            }
            let t156: [uint32x4_t; 2] = [vandq_u32(live[0], x6[0]), vandq_u32(live[1], x6[1])];
            {
                let t157: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[1].as_ptr()))];
                if vgetq_lane_u32::<0>(t156[0]) != 0 { (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<0>(t157[0])); }
                if vgetq_lane_u32::<1>(t156[0]) != 0 { (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<1>(t157[0])); }
                if vgetq_lane_u32::<2>(t156[0]) != 0 { (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<2>(t157[0])); }
                if vgetq_lane_u32::<3>(t156[0]) != 0 { (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<3>(t157[0])); }
                if vgetq_lane_u32::<0>(t156[1]) != 0 { (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<0>(t157[1])); }
                if vgetq_lane_u32::<1>(t156[1]) != 0 { (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<1>(t157[1])); }
                if vgetq_lane_u32::<2>(t156[1]) != 0 { (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<2>(t157[1])); }
                if vgetq_lane_u32::<3>(t156[1]) != 0 { (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(vgetq_lane_u32::<3>(t157[1])); }
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
    let x2: *const u8 = *argv.add(2);
    let arg3: *const u8 = (*argv.add(3) as *const *const u8).read_unaligned();
    let x4: u32 = (*argv.add(4) as *const u32).read_unaligned();
    let mut t158: bool = !(arg3).is_null();
    if t158 {
        let mut t159: bool = ((l.group[0]) >= ((arg3.wrapping_add((((x4) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t159 {
            t159 = ((l.group[1]) >= ((arg3.wrapping_add(((((x4).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t160: bool = t159;
        if !t160 {
            t160 = ((l.group[2]) >= ((arg3.wrapping_add(((((x4).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t158 = t160;
    }
    if t158 {
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
        let t161: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t161;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1sg_p(l, lid0, live, arg0, arg1, x2);
        }
    }
}
