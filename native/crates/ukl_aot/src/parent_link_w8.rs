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
            let t22 = bits(&t20);
            if t22.count_ones() <= 2 {
                let mut t23 = t22;
                while t23 != 0 {
                    let j = t23.trailing_zeros() as usize;
                    t23 &= t23 - 1;
                    let t24 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x4);
                    let mut x4_j: u32 = t24[j];
                    let mut x5_j: u32 = x5;
                    loop {
                        let mut t25: bool = ((x5_j) < (7u32));
                        if t25 {
                            t25 = ((x4_j) != (4294967295u32));
                        }
                        let mut t26: bool = t25;
                        if t26 {
                            t26 = ((((x4_j) & (2147483648u32))) != (0u32));
                        }
                        if !(t26) { break; }
                        x4_j = (arg0.wrapping_add((((((x4_j) & (2147483647u32))) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
                        let t27 = x5_j;
                        x5_j = (x5_j).wrapping_add(1u32);
                    }
                    let mut t28 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x4);
                    t28[j] = x4_j as u32;
                    x4 = core::mem::transmute::<[u32; 8], [uint32x4_t; 2]>(t28);
                }
                break;
            }
            // Lanes part: each its own rounds.
            let t30 = [vdupq_n_u32(4294967295u32); 2];
            let t31: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t30[0])), vmvnq_u32(vceqq_u32(x4[1], t30[1]))];
            let t32: [uint32x4_t; 2] = [vandq_u32(vdupq_n_u32(if ((x5) < (7u32)) { u32::MAX } else { 0 }), t31[0]), vandq_u32(vdupq_n_u32(if ((x5) < (7u32)) { u32::MAX } else { 0 }), t31[1])];
            let t33 = [vdupq_n_u32(2147483648u32); 2];
            let t34: [uint32x4_t; 2] = [vandq_u32(x4[0], t33[0]), vandq_u32(x4[1], t33[1])];
            let t35 = [vdupq_n_u32(0u32); 2];
            let t36: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t34[0], t35[0])), vmvnq_u32(vceqq_u32(t34[1], t35[1]))];
            let t37: [uint32x4_t; 2] = [vandq_u32(t32[0], t36[0]), vandq_u32(t32[1], t36[1])];
            t20 = [vandq_u32(t20[0], t37[0]), vandq_u32(t20[1], t37[1])];
            if !(any(&t20)) { break; }
            let t39 = [vdupq_n_u32(2147483647u32); 2];
            let t40: [uint32x4_t; 2] = [vandq_u32(x4[0], t39[0]), vandq_u32(x4[1], t39[1])];
            let t41 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(t20[0], t40[0]), vandq_u32(t20[1], t40[1]))));
            let t42: [uint32x4_t; 2] = [vbslq_u32(t20[0], t40[0], t41), vbslq_u32(t20[1], t40[1], t41)];
            let t43 = arg0 as *const u32;
            let mut t44: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t44[0] = vld1q_lane_u32::<0>(t43.add(vgetq_lane_u32::<0>(t42[0]) as usize), t44[0]);
            t44[0] = vld1q_lane_u32::<1>(t43.add(vgetq_lane_u32::<1>(t42[0]) as usize), t44[0]);
            t44[0] = vld1q_lane_u32::<2>(t43.add(vgetq_lane_u32::<2>(t42[0]) as usize), t44[0]);
            t44[0] = vld1q_lane_u32::<3>(t43.add(vgetq_lane_u32::<3>(t42[0]) as usize), t44[0]);
            t44[1] = vld1q_lane_u32::<0>(t43.add(vgetq_lane_u32::<0>(t42[1]) as usize), t44[1]);
            t44[1] = vld1q_lane_u32::<1>(t43.add(vgetq_lane_u32::<1>(t42[1]) as usize), t44[1]);
            t44[1] = vld1q_lane_u32::<2>(t43.add(vgetq_lane_u32::<2>(t42[1]) as usize), t44[1]);
            t44[1] = vld1q_lane_u32::<3>(t43.add(vgetq_lane_u32::<3>(t42[1]) as usize), t44[1]);
            x4 = [vbslq_u32(t20[0], t44[0], x4[0]), vbslq_u32(t20[1], t44[1], x4[1])];
            let t45: [uint32x4_t; 2] = [vandq_u32(t20[0], live[0]), vandq_u32(t20[1], live[1])];
            let t46 = x5;
            x5 = (x5).wrapping_add(1u32);
        }
        let t47 = [vdupq_n_u32(4294967295u32); 2];
        let t48: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t47[0])), vmvnq_u32(vceqq_u32(x4[1], t47[1]))];
        let t49 = [vdupq_n_u32(2147483648u32); 2];
        let t50: [uint32x4_t; 2] = [vandq_u32(x4[0], t49[0]), vandq_u32(x4[1], t49[1])];
        let t51 = [vdupq_n_u32(0u32); 2];
        let t52: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t50[0], t51[0])), vmvnq_u32(vceqq_u32(t50[1], t51[1]))];
        let t53: [uint32x4_t; 2] = [vandq_u32(t48[0], t52[0]), vandq_u32(t48[1], t52[1])];
        x6 = t53;
        let mut t54: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t55: [uint32x4_t; 2] = [vandq_u32(live[0], x6[0]), vandq_u32(live[1], x6[1])];
        let t56: [uint32x4_t; 2] = [vbicq_u32(live[0], x6[0]), vbicq_u32(live[1], x6[1])];
        {
            t54 = [vbslq_u32(t55[0], t47[0], t54[0]), vbslq_u32(t55[1], t47[1], t54[1])];
        }
        {
            t54 = [vbslq_u32(t56[0], x4[0], t54[0]), vbslq_u32(t56[1], x4[1], t54[1])];
        }
        let t57 = (arg1 as *mut u32).add((gid0) as usize);
        if all(&live) {
            vst1q_u32(t57.add(0), t54[0]);
            vst1q_u32(t57.add(4), t54[1]);
        } else {
            let mut t58w = 0u32;
            let t58 = core::hint::black_box(&raw mut t58w);
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t57.add(0), t58).write(vgetq_lane_u32::<0>(t54[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t57.add(1), t58).write(vgetq_lane_u32::<1>(t54[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t57.add(2), t58).write(vgetq_lane_u32::<2>(t54[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t57.add(3), t58).write(vgetq_lane_u32::<3>(t54[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t57.add(4), t58).write(vgetq_lane_u32::<0>(t54[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t57.add(5), t58).write(vgetq_lane_u32::<1>(t54[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t57.add(6), t58).write(vgetq_lane_u32::<2>(t54[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t57.add(7), t58).write(vgetq_lane_u32::<3>(t54[1]));
        }
        let t59: [uint32x4_t; 2] = [vandq_u32(live[0], x6[0]), vandq_u32(live[1], x6[1])];
        {
            let t60: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[1].as_ptr()))];
            let t61 = bits(&t59);
            if t61 != 0 {
                let j = 31 - t61.leading_zeros() as usize;
                (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(t60)[j]);
            }
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
        let t62 = (arg0 as *const u32).add((gid0) as usize);
        let t63: [uint32x4_t; 2] = [vld1q_u32(t62.add(0)), vld1q_u32(t62.add(4))];
        x4 = t63;
        x5 = 0u32;
        let mut t82 = live;
        loop {
            let t84 = bits(&t82);
            if t84.count_ones() <= 2 {
                let mut t85 = t84;
                while t85 != 0 {
                    let j = t85.trailing_zeros() as usize;
                    t85 &= t85 - 1;
                    let t86 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x4);
                    let mut x4_j: u32 = t86[j];
                    let mut x5_j: u32 = x5;
                    loop {
                        let mut t87: bool = ((x5_j) < (7u32));
                        if t87 {
                            t87 = ((x4_j) != (4294967295u32));
                        }
                        let mut t88: bool = t87;
                        if t88 {
                            t88 = ((((x4_j) & (2147483648u32))) != (0u32));
                        }
                        if !(t88) { break; }
                        x4_j = (arg0.wrapping_add((((((x4_j) & (2147483647u32))) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
                        let t89 = x5_j;
                        x5_j = (x5_j).wrapping_add(1u32);
                    }
                    let mut t90 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x4);
                    t90[j] = x4_j as u32;
                    x4 = core::mem::transmute::<[u32; 8], [uint32x4_t; 2]>(t90);
                }
                break;
            }
            // Lanes part: each its own rounds.
            let t92 = [vdupq_n_u32(4294967295u32); 2];
            let t93: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t92[0])), vmvnq_u32(vceqq_u32(x4[1], t92[1]))];
            let t94: [uint32x4_t; 2] = [vandq_u32(vdupq_n_u32(if ((x5) < (7u32)) { u32::MAX } else { 0 }), t93[0]), vandq_u32(vdupq_n_u32(if ((x5) < (7u32)) { u32::MAX } else { 0 }), t93[1])];
            let t95 = [vdupq_n_u32(2147483648u32); 2];
            let t96: [uint32x4_t; 2] = [vandq_u32(x4[0], t95[0]), vandq_u32(x4[1], t95[1])];
            let t97 = [vdupq_n_u32(0u32); 2];
            let t98: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t96[0], t97[0])), vmvnq_u32(vceqq_u32(t96[1], t97[1]))];
            let t99: [uint32x4_t; 2] = [vandq_u32(t94[0], t98[0]), vandq_u32(t94[1], t98[1])];
            t82 = [vandq_u32(t82[0], t99[0]), vandq_u32(t82[1], t99[1])];
            if !(any(&t82)) { break; }
            let t101 = [vdupq_n_u32(2147483647u32); 2];
            let t102: [uint32x4_t; 2] = [vandq_u32(x4[0], t101[0]), vandq_u32(x4[1], t101[1])];
            let t103: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t104: [uint32x4_t; 2] = [vbslq_u32(t82[0], t102[0], t103[0]), vbslq_u32(t82[1], t102[1], t103[1])];
            let t105 = arg0 as *const u32;
            let mut t106: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t106[0] = vld1q_lane_u32::<0>(t105.add(vgetq_lane_u32::<0>(t104[0]) as usize), t106[0]);
            t106[0] = vld1q_lane_u32::<1>(t105.add(vgetq_lane_u32::<1>(t104[0]) as usize), t106[0]);
            t106[0] = vld1q_lane_u32::<2>(t105.add(vgetq_lane_u32::<2>(t104[0]) as usize), t106[0]);
            t106[0] = vld1q_lane_u32::<3>(t105.add(vgetq_lane_u32::<3>(t104[0]) as usize), t106[0]);
            t106[1] = vld1q_lane_u32::<0>(t105.add(vgetq_lane_u32::<0>(t104[1]) as usize), t106[1]);
            t106[1] = vld1q_lane_u32::<1>(t105.add(vgetq_lane_u32::<1>(t104[1]) as usize), t106[1]);
            t106[1] = vld1q_lane_u32::<2>(t105.add(vgetq_lane_u32::<2>(t104[1]) as usize), t106[1]);
            t106[1] = vld1q_lane_u32::<3>(t105.add(vgetq_lane_u32::<3>(t104[1]) as usize), t106[1]);
            x4 = [vbslq_u32(t82[0], t106[0], x4[0]), vbslq_u32(t82[1], t106[1], x4[1])];
            let t107: [uint32x4_t; 2] = [vandq_u32(t82[0], live[0]), vandq_u32(t82[1], live[1])];
            let t108 = x5;
            x5 = (x5).wrapping_add(1u32);
        }
        let t109 = [vdupq_n_u32(4294967295u32); 2];
        let t110: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t109[0])), vmvnq_u32(vceqq_u32(x4[1], t109[1]))];
        let t111 = [vdupq_n_u32(2147483648u32); 2];
        let t112: [uint32x4_t; 2] = [vandq_u32(x4[0], t111[0]), vandq_u32(x4[1], t111[1])];
        let t113 = [vdupq_n_u32(0u32); 2];
        let t114: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t112[0], t113[0])), vmvnq_u32(vceqq_u32(t112[1], t113[1]))];
        let t115: [uint32x4_t; 2] = [vandq_u32(t110[0], t114[0]), vandq_u32(t110[1], t114[1])];
        x6 = t115;
        let mut t116: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t117: [uint32x4_t; 2] = [vmvnq_u32(x6[0]), vmvnq_u32(x6[1])];
        {
            t116 = [vbslq_u32(x6[0], t109[0], t116[0]), vbslq_u32(x6[1], t109[1], t116[1])];
        }
        {
            t116 = [vbslq_u32(t117[0], x4[0], t116[0]), vbslq_u32(t117[1], x4[1], t116[1])];
        }
        let t118 = (arg1 as *mut u32).add((gid0) as usize);
        vst1q_u32(t118.add(0), t116[0]);
        vst1q_u32(t118.add(4), t116[1]);
        {
            let t119: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[1].as_ptr()))];
            let t120 = bits(&x6);
            if t120 != 0 {
                let j = 31 - t120.leading_zeros() as usize;
                (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(t119)[j]);
            }
        }
    } else if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= (((x2 as *const u32).read_unaligned()) as u64)) {
        return;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t121: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t122 = [vdupq_n_u32((x2 as *const u32).read_unaligned()); 2];
        let t123: [uint32x4_t; 2] = [vcgeq_u32(t121[0], t122[0]), vcgeq_u32(t121[1], t122[1])];
        let t124: [uint32x4_t; 2] = [vandq_u32(live[0], t123[0]), vandq_u32(live[1], t123[1])];
        {
            live = [vbicq_u32(live[0], t124[0]), vbicq_u32(live[1], t124[1])];
        }
        if any(&live) {
            let mut t125: [uint32x4_t; 2];
            if all(&live) {
                let t126 = (arg0 as *const u32).add((gid0) as usize);
                t125 = [vld1q_u32(t126.add(0)), vld1q_u32(t126.add(4))];
            } else {
                let t127 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t121[0]), vandq_u32(live[1], t121[1]))));
                let t128: [uint32x4_t; 2] = [vbslq_u32(live[0], t121[0], t127), vbslq_u32(live[1], t121[1], t127)];
                let t129 = arg0 as *const u32;
                let mut t130: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                t130[0] = vld1q_lane_u32::<0>(t129.add(vgetq_lane_u32::<0>(t128[0]) as usize), t130[0]);
                t130[0] = vld1q_lane_u32::<1>(t129.add(vgetq_lane_u32::<1>(t128[0]) as usize), t130[0]);
                t130[0] = vld1q_lane_u32::<2>(t129.add(vgetq_lane_u32::<2>(t128[0]) as usize), t130[0]);
                t130[0] = vld1q_lane_u32::<3>(t129.add(vgetq_lane_u32::<3>(t128[0]) as usize), t130[0]);
                t130[1] = vld1q_lane_u32::<0>(t129.add(vgetq_lane_u32::<0>(t128[1]) as usize), t130[1]);
                t130[1] = vld1q_lane_u32::<1>(t129.add(vgetq_lane_u32::<1>(t128[1]) as usize), t130[1]);
                t130[1] = vld1q_lane_u32::<2>(t129.add(vgetq_lane_u32::<2>(t128[1]) as usize), t130[1]);
                t130[1] = vld1q_lane_u32::<3>(t129.add(vgetq_lane_u32::<3>(t128[1]) as usize), t130[1]);
                t125 = t130;
            }
            x4 = t125;
            x5 = 0u32;
            let mut t140 = live;
            loop {
                let t142 = bits(&t140);
                if t142.count_ones() <= 2 {
                    let mut t143 = t142;
                    while t143 != 0 {
                        let j = t143.trailing_zeros() as usize;
                        t143 &= t143 - 1;
                        let t144 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x4);
                        let mut x4_j: u32 = t144[j];
                        let mut x5_j: u32 = x5;
                        loop {
                            let mut t145: bool = ((x5_j) < (7u32));
                            if t145 {
                                t145 = ((x4_j) != (4294967295u32));
                            }
                            let mut t146: bool = t145;
                            if t146 {
                                t146 = ((((x4_j) & (2147483648u32))) != (0u32));
                            }
                            if !(t146) { break; }
                            x4_j = (arg0.wrapping_add((((((x4_j) & (2147483647u32))) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
                            let t147 = x5_j;
                            x5_j = (x5_j).wrapping_add(1u32);
                        }
                        let mut t148 = core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(x4);
                        t148[j] = x4_j as u32;
                        x4 = core::mem::transmute::<[u32; 8], [uint32x4_t; 2]>(t148);
                    }
                    break;
                }
                // Lanes part: each its own rounds.
                let t150 = [vdupq_n_u32(4294967295u32); 2];
                let t151: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t150[0])), vmvnq_u32(vceqq_u32(x4[1], t150[1]))];
                let t152: [uint32x4_t; 2] = [vandq_u32(vdupq_n_u32(if ((x5) < (7u32)) { u32::MAX } else { 0 }), t151[0]), vandq_u32(vdupq_n_u32(if ((x5) < (7u32)) { u32::MAX } else { 0 }), t151[1])];
                let t153 = [vdupq_n_u32(2147483648u32); 2];
                let t154: [uint32x4_t; 2] = [vandq_u32(x4[0], t153[0]), vandq_u32(x4[1], t153[1])];
                let t155 = [vdupq_n_u32(0u32); 2];
                let t156: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t154[0], t155[0])), vmvnq_u32(vceqq_u32(t154[1], t155[1]))];
                let t157: [uint32x4_t; 2] = [vandq_u32(t152[0], t156[0]), vandq_u32(t152[1], t156[1])];
                t140 = [vandq_u32(t140[0], t157[0]), vandq_u32(t140[1], t157[1])];
                if !(any(&t140)) { break; }
                let t159 = [vdupq_n_u32(2147483647u32); 2];
                let t160: [uint32x4_t; 2] = [vandq_u32(x4[0], t159[0]), vandq_u32(x4[1], t159[1])];
                let t161 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(t140[0], t160[0]), vandq_u32(t140[1], t160[1]))));
                let t162: [uint32x4_t; 2] = [vbslq_u32(t140[0], t160[0], t161), vbslq_u32(t140[1], t160[1], t161)];
                let t163 = arg0 as *const u32;
                let mut t164: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                t164[0] = vld1q_lane_u32::<0>(t163.add(vgetq_lane_u32::<0>(t162[0]) as usize), t164[0]);
                t164[0] = vld1q_lane_u32::<1>(t163.add(vgetq_lane_u32::<1>(t162[0]) as usize), t164[0]);
                t164[0] = vld1q_lane_u32::<2>(t163.add(vgetq_lane_u32::<2>(t162[0]) as usize), t164[0]);
                t164[0] = vld1q_lane_u32::<3>(t163.add(vgetq_lane_u32::<3>(t162[0]) as usize), t164[0]);
                t164[1] = vld1q_lane_u32::<0>(t163.add(vgetq_lane_u32::<0>(t162[1]) as usize), t164[1]);
                t164[1] = vld1q_lane_u32::<1>(t163.add(vgetq_lane_u32::<1>(t162[1]) as usize), t164[1]);
                t164[1] = vld1q_lane_u32::<2>(t163.add(vgetq_lane_u32::<2>(t162[1]) as usize), t164[1]);
                t164[1] = vld1q_lane_u32::<3>(t163.add(vgetq_lane_u32::<3>(t162[1]) as usize), t164[1]);
                x4 = [vbslq_u32(t140[0], t164[0], x4[0]), vbslq_u32(t140[1], t164[1], x4[1])];
                let t165: [uint32x4_t; 2] = [vandq_u32(t140[0], live[0]), vandq_u32(t140[1], live[1])];
                let t166 = x5;
                x5 = (x5).wrapping_add(1u32);
            }
            let t167 = [vdupq_n_u32(4294967295u32); 2];
            let t168: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x4[0], t167[0])), vmvnq_u32(vceqq_u32(x4[1], t167[1]))];
            let t169 = [vdupq_n_u32(2147483648u32); 2];
            let t170: [uint32x4_t; 2] = [vandq_u32(x4[0], t169[0]), vandq_u32(x4[1], t169[1])];
            let t171 = [vdupq_n_u32(0u32); 2];
            let t172: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t170[0], t171[0])), vmvnq_u32(vceqq_u32(t170[1], t171[1]))];
            let t173: [uint32x4_t; 2] = [vandq_u32(t168[0], t172[0]), vandq_u32(t168[1], t172[1])];
            x6 = t173;
            let mut t174: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t175: [uint32x4_t; 2] = [vandq_u32(live[0], x6[0]), vandq_u32(live[1], x6[1])];
            let t176: [uint32x4_t; 2] = [vbicq_u32(live[0], x6[0]), vbicq_u32(live[1], x6[1])];
            {
                t174 = [vbslq_u32(t175[0], t167[0], t174[0]), vbslq_u32(t175[1], t167[1], t174[1])];
            }
            {
                t174 = [vbslq_u32(t176[0], x4[0], t174[0]), vbslq_u32(t176[1], x4[1], t174[1])];
            }
            let t177 = (arg1 as *mut u32).add((gid0) as usize);
            if all(&live) {
                vst1q_u32(t177.add(0), t174[0]);
                vst1q_u32(t177.add(4), t174[1]);
            } else {
                let mut t178w = 0u32;
                let t178 = core::hint::black_box(&raw mut t178w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t177.add(0), t178).write(vgetq_lane_u32::<0>(t174[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t177.add(1), t178).write(vgetq_lane_u32::<1>(t174[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t177.add(2), t178).write(vgetq_lane_u32::<2>(t174[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t177.add(3), t178).write(vgetq_lane_u32::<3>(t174[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t177.add(4), t178).write(vgetq_lane_u32::<0>(t174[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t177.add(5), t178).write(vgetq_lane_u32::<1>(t174[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t177.add(6), t178).write(vgetq_lane_u32::<2>(t174[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t177.add(7), t178).write(vgetq_lane_u32::<3>(t174[1]));
            }
            let t179: [uint32x4_t; 2] = [vandq_u32(live[0], x6[0]), vandq_u32(live[1], x6[1])];
            {
                let t180: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0.wrapping_add(1u32)), vld1q_u32(LANE[1].as_ptr()))];
                let t181 = bits(&t179);
                if t181 != 0 {
                    let j = 31 - t181.leading_zeros() as usize;
                    (arg1.wrapping_add(((((x2 as *const u32).read_unaligned()) as u64)).wrapping_mul(4u64) as usize) as *mut u32).write_unaligned(core::mem::transmute::<[uint32x4_t; 2], [u32; 8]>(t180)[j]);
                }
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
    let mut t182: bool = !(arg3).is_null();
    if t182 {
        let mut t183: bool = ((l.group[0]) >= ((arg3.wrapping_add((((x4) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t183 {
            t183 = ((l.group[1]) >= ((arg3.wrapping_add(((((x4).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t184: bool = t183;
        if !t184 {
            t184 = ((l.group[2]) >= ((arg3.wrapping_add(((((x4).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t182 = t184;
    }
    if t182 {
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
        let t185: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t185;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1sg_p(l, lid0, live, arg0, arg1, x2);
        }
    }
}
