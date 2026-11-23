//! `parent_back` at 8 lanes (2 NEON registers), written by rerail-ukl-vec's Rust (`rs.rs`) from its vector tape: do
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
unsafe fn b0_r0r1r2r3sgi_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, x32: *const u8, x56: u32) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x32 as *const u32).read_unaligned();
    let mut live = k;
    b1_r0r1r2sg_p(l, lid0, live, arg0, arg1, arg2, a0);
}

#[inline(always)]
unsafe fn b0_r0r1r2r3sgi_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, x32: *const u8, x56: u32) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x32 as *const u32).read_unaligned();
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    b1_r0r1r2sg_f(l, lid0, arg0, arg1, arg2, a0);
}

#[inline(always)]
unsafe fn b1_r0r1r2sg_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, arg2: *const u8, x24: u32) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x40: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x56: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x64: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x72: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x80: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let t1: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t2 = [vdupq_n_u32(x24); 2];
    let t3: [uint32x4_t; 2] = [vcgeq_u32(t1[0], t2[0]), vcgeq_u32(t1[1], t2[1])];
    let t4: [uint32x4_t; 2] = [vandq_u32(live[0], t3[0]), vandq_u32(live[1], t3[1])];
    if any(&t4) {
        live = [vbicq_u32(live[0], t4[0]), vbicq_u32(live[1], t4[1])];
    }
    if any(&live) {
        let t5 = b2_r0r1g_p(l, lid0, live, arg0, arg1);
        x40 = t5;
        let t6 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
        let t7: [uint64x2_t; 4] = [vandq_u64(x40[0], t6[0]), vandq_u64(x40[1], t6[1]), vandq_u64(x40[2], t6[2]), vandq_u64(x40[3], t6[3])];
        let t8 = [vdupq_n_u64(0u64); 4];
        let t9: [uint64x2_t; 4] = [vceqq_u64(t7[0], t8[0]), vceqq_u64(t7[1], t8[1]), vceqq_u64(t7[2], t8[2]), vceqq_u64(t7[3], t8[3])];
        let t10: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t9[0]), vmovn_u64(t9[1])), vcombine_u32(vmovn_u64(t9[2]), vmovn_u64(t9[3]))];
        let t11: [uint32x4_t; 2] = [vmvnq_u32(t10[0]), vmvnq_u32(t10[1])];
        let mut t12: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t13: [uint32x4_t; 2] = [vandq_u32(live[0], t11[0]), vandq_u32(live[1], t11[1])];
        let t14: [uint32x4_t; 2] = [vbicq_u32(live[0], t11[0]), vbicq_u32(live[1], t11[1])];
        {
            let t15: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x40[0]), vmovn_u64(x40[1])), vcombine_u32(vmovn_u64(x40[2]), vmovn_u64(x40[3]))];
            let t16 = [vdupq_n_u32(2147483648u32); 2];
            let t17: [uint32x4_t; 2] = [vorrq_u32(t16[0], t15[0]), vorrq_u32(t16[1], t15[1])];
            t12 = [vbslq_u32(t13[0], t17[0], t12[0]), vbslq_u32(t13[1], t17[1], t12[1])];
        }
        {
            let t18: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x40[0]), vmovn_u64(x40[1])), vcombine_u32(vmovn_u64(x40[2]), vmovn_u64(x40[3]))];
            t12 = [vbslq_u32(t14[0], t18[0], t12[0]), vbslq_u32(t14[1], t18[1], t12[1])];
        }
        x48 = t12;
        b3_ag4r2l_p(l, lid0, live, arg2, x48);
        let t19 = [vdupq_n_u32(0u32); 2];
        x56 = t19;
        let mut t20 = live;
        loop {
            // Lanes part: each its own rounds.
            let t22 = [vdupq_n_u32(7u32); 2];
            let t23: [uint32x4_t; 2] = [vcltq_u32(x56[0], t22[0]), vcltq_u32(x56[1], t22[1])];
            let t24 = [vdupq_n_u32(2147483648u32); 2];
            let t25: [uint32x4_t; 2] = [vandq_u32(x48[0], t24[0]), vandq_u32(x48[1], t24[1])];
            let t26: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t25[0], t19[0])), vmvnq_u32(vceqq_u32(t25[1], t19[1]))];
            let t27 = [vdupq_n_u32(4294967294u32); 2];
            let t28: [uint32x4_t; 2] = [vcltq_u32(x48[0], t27[0]), vcltq_u32(x48[1], t27[1])];
            let t29: [uint32x4_t; 2] = [vandq_u32(t26[0], t28[0]), vandq_u32(t26[1], t28[1])];
            let t30: [uint32x4_t; 2] = [vandq_u32(t23[0], t29[0]), vandq_u32(t23[1], t29[1])];
            t20 = [vandq_u32(t20[0], t30[0]), vandq_u32(t20[1], t30[1])];
            if !(any(&t20)) { break; }
            let t32 = [vdupq_n_u32((2147483648u32).wrapping_sub(1u32)); 2];
            let t33: [uint32x4_t; 2] = [vandq_u32(x48[0], t32[0]), vandq_u32(x48[1], t32[1])];
            x64 = t33;
            let t34 = b4_al4r2_p(l, lid0, t20, arg2, x64);
            x72 = t34;
            let t35: [uint32x4_t; 2] = [vceqq_u32(x72[0], t27[0]), vceqq_u32(x72[1], t27[1])];
            let t36: [uint32x4_t; 2] = [vandq_u32(t20[0], t35[0]), vandq_u32(t20[1], t35[1])];
            if any(&t36) {
                let t37 = b2_r0r1l_p(l, lid0, t36, arg0, arg1, x64);
                x80 = t37;
                let t38 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                let t39: [uint64x2_t; 4] = [vandq_u64(x80[0], t38[0]), vandq_u64(x80[1], t38[1]), vandq_u64(x80[2], t38[2]), vandq_u64(x80[3], t38[3])];
                let t40: [uint64x2_t; 4] = [vceqq_u64(t39[0], t8[0]), vceqq_u64(t39[1], t8[1]), vceqq_u64(t39[2], t8[2]), vceqq_u64(t39[3], t8[3])];
                let t41: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t40[0]), vmovn_u64(t40[1])), vcombine_u32(vmovn_u64(t40[2]), vmovn_u64(t40[3]))];
                let t42: [uint32x4_t; 2] = [vmvnq_u32(t41[0]), vmvnq_u32(t41[1])];
                let mut t43: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t44: [uint32x4_t; 2] = [vandq_u32(t36[0], t42[0]), vandq_u32(t36[1], t42[1])];
                let t45: [uint32x4_t; 2] = [vbicq_u32(t36[0], t42[0]), vbicq_u32(t36[1], t42[1])];
                {
                    let t46: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x80[0]), vmovn_u64(x80[1])), vcombine_u32(vmovn_u64(x80[2]), vmovn_u64(x80[3]))];
                    let t47: [uint32x4_t; 2] = [vorrq_u32(t24[0], t46[0]), vorrq_u32(t24[1], t46[1])];
                    t43 = [vbslq_u32(t44[0], t47[0], t43[0]), vbslq_u32(t44[1], t47[1], t43[1])];
                }
                {
                    let t48: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x80[0]), vmovn_u64(x80[1])), vcombine_u32(vmovn_u64(x80[2]), vmovn_u64(x80[3]))];
                    t43 = [vbslq_u32(t45[0], t48[0], t43[0]), vbslq_u32(t45[1], t48[1], t43[1])];
                }
                x72 = [vbslq_u32(t36[0], t43[0], x72[0]), vbslq_u32(t36[1], t43[1], x72[1])];
            }
            x48 = [vbslq_u32(t20[0], x72[0], x48[0]), vbslq_u32(t20[1], x72[1], x48[1])];
            let t49: [uint32x4_t; 2] = [vandq_u32(t20[0], live[0]), vandq_u32(t20[1], live[1])];
            let t50 = [vdupq_n_u32(1u32); 2];
            let t51: [uint32x4_t; 2] = [vaddq_u32(x56[0], t50[0]), vaddq_u32(x56[1], t50[1])];
            let t52 = x56;
            x56 = t51;
        }
        let t53 = [vdupq_n_u32(2147483648u32); 2];
        let t54: [uint32x4_t; 2] = [vandq_u32(x48[0], t53[0]), vandq_u32(x48[1], t53[1])];
        let t55: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t54[0], t19[0])), vmvnq_u32(vceqq_u32(t54[1], t19[1]))];
        let t56 = [vdupq_n_u32(4294967294u32); 2];
        let t57: [uint32x4_t; 2] = [vcltq_u32(x48[0], t56[0]), vcltq_u32(x48[1], t56[1])];
        let t58: [uint32x4_t; 2] = [vandq_u32(t55[0], t57[0]), vandq_u32(t55[1], t57[1])];
        let t59: [uint32x4_t; 2] = [vandq_u32(live[0], t58[0]), vandq_u32(live[1], t58[1])];
        if any(&t59) {
            let t60 = [vdupq_n_u32((2147483648u32).wrapping_sub(1u32)); 2];
            let t61: [uint32x4_t; 2] = [vandq_u32(x48[0], t60[0]), vandq_u32(x48[1], t60[1])];
            let t62 = b5_r0r1l_p(l, lid0, t59, arg0, arg1, t61);
            x48 = [vbslq_u32(t59[0], t62[0], x48[0]), vbslq_u32(t59[1], t62[1], x48[1])];
        }
        b3_ag4r2l_p(l, lid0, live, arg2, x48);
    }
}

#[inline(always)]
unsafe fn b1_r0r1r2sg_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, arg2: *const u8, x24: u32) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x40: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x56: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x64: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x72: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x80: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if ((gid0) as u64 + 7) < ((x24) as u64) {
        let t63 = b2_r0r1g_f(l, lid0, arg0, arg1);
        x40 = t63;
        let t64 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
        let t65: [uint64x2_t; 4] = [vandq_u64(x40[0], t64[0]), vandq_u64(x40[1], t64[1]), vandq_u64(x40[2], t64[2]), vandq_u64(x40[3], t64[3])];
        let t66 = [vdupq_n_u64(0u64); 4];
        let t67: [uint64x2_t; 4] = [vceqq_u64(t65[0], t66[0]), vceqq_u64(t65[1], t66[1]), vceqq_u64(t65[2], t66[2]), vceqq_u64(t65[3], t66[3])];
        let t68: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t67[0]), vmovn_u64(t67[1])), vcombine_u32(vmovn_u64(t67[2]), vmovn_u64(t67[3]))];
        let t69: [uint32x4_t; 2] = [vmvnq_u32(t68[0]), vmvnq_u32(t68[1])];
        let mut t70: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        let t71: [uint32x4_t; 2] = [vmvnq_u32(t69[0]), vmvnq_u32(t69[1])];
        {
            let t72: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x40[0]), vmovn_u64(x40[1])), vcombine_u32(vmovn_u64(x40[2]), vmovn_u64(x40[3]))];
            let t73 = [vdupq_n_u32(2147483648u32); 2];
            let t74: [uint32x4_t; 2] = [vorrq_u32(t73[0], t72[0]), vorrq_u32(t73[1], t72[1])];
            t70 = [vbslq_u32(t69[0], t74[0], t70[0]), vbslq_u32(t69[1], t74[1], t70[1])];
        }
        {
            let t75: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x40[0]), vmovn_u64(x40[1])), vcombine_u32(vmovn_u64(x40[2]), vmovn_u64(x40[3]))];
            t70 = [vbslq_u32(t71[0], t75[0], t70[0]), vbslq_u32(t71[1], t75[1], t70[1])];
        }
        x48 = t70;
        b3_ag4r2l_f(l, lid0, arg2, x48);
        let t76 = [vdupq_n_u32(0u32); 2];
        x56 = t76;
        let mut t86 = live;
        loop {
            // Lanes part: each its own rounds.
            let t88 = [vdupq_n_u32(7u32); 2];
            let t89: [uint32x4_t; 2] = [vcltq_u32(x56[0], t88[0]), vcltq_u32(x56[1], t88[1])];
            let t90 = [vdupq_n_u32(2147483648u32); 2];
            let t91: [uint32x4_t; 2] = [vandq_u32(x48[0], t90[0]), vandq_u32(x48[1], t90[1])];
            let t92: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t91[0], t76[0])), vmvnq_u32(vceqq_u32(t91[1], t76[1]))];
            let t93 = [vdupq_n_u32(4294967294u32); 2];
            let t94: [uint32x4_t; 2] = [vcltq_u32(x48[0], t93[0]), vcltq_u32(x48[1], t93[1])];
            let t95: [uint32x4_t; 2] = [vandq_u32(t92[0], t94[0]), vandq_u32(t92[1], t94[1])];
            let t96: [uint32x4_t; 2] = [vandq_u32(t89[0], t95[0]), vandq_u32(t89[1], t95[1])];
            t86 = [vandq_u32(t86[0], t96[0]), vandq_u32(t86[1], t96[1])];
            if !(any(&t86)) { break; }
            let t98 = [vdupq_n_u32((2147483648u32).wrapping_sub(1u32)); 2];
            let t99: [uint32x4_t; 2] = [vandq_u32(x48[0], t98[0]), vandq_u32(x48[1], t98[1])];
            x64 = t99;
            let t100 = b4_al4r2_p_v2(l, lid0, t86, arg2, x64);
            x72 = t100;
            let t101: [uint32x4_t; 2] = [vceqq_u32(x72[0], t93[0]), vceqq_u32(x72[1], t93[1])];
            let t102: [uint32x4_t; 2] = [vandq_u32(t86[0], t101[0]), vandq_u32(t86[1], t101[1])];
            if any(&t102) {
                let t103 = b2_r0r1l_p(l, lid0, t102, arg0, arg1, x64);
                x80 = t103;
                let t104 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                let t105: [uint64x2_t; 4] = [vandq_u64(x80[0], t104[0]), vandq_u64(x80[1], t104[1]), vandq_u64(x80[2], t104[2]), vandq_u64(x80[3], t104[3])];
                let t106: [uint64x2_t; 4] = [vceqq_u64(t105[0], t66[0]), vceqq_u64(t105[1], t66[1]), vceqq_u64(t105[2], t66[2]), vceqq_u64(t105[3], t66[3])];
                let t107: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t106[0]), vmovn_u64(t106[1])), vcombine_u32(vmovn_u64(t106[2]), vmovn_u64(t106[3]))];
                let t108: [uint32x4_t; 2] = [vmvnq_u32(t107[0]), vmvnq_u32(t107[1])];
                let mut t109: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                let t110: [uint32x4_t; 2] = [vandq_u32(t102[0], t108[0]), vandq_u32(t102[1], t108[1])];
                let t111: [uint32x4_t; 2] = [vbicq_u32(t102[0], t108[0]), vbicq_u32(t102[1], t108[1])];
                {
                    let t112: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x80[0]), vmovn_u64(x80[1])), vcombine_u32(vmovn_u64(x80[2]), vmovn_u64(x80[3]))];
                    let t113: [uint32x4_t; 2] = [vorrq_u32(t90[0], t112[0]), vorrq_u32(t90[1], t112[1])];
                    t109 = [vbslq_u32(t110[0], t113[0], t109[0]), vbslq_u32(t110[1], t113[1], t109[1])];
                }
                {
                    let t114: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x80[0]), vmovn_u64(x80[1])), vcombine_u32(vmovn_u64(x80[2]), vmovn_u64(x80[3]))];
                    t109 = [vbslq_u32(t111[0], t114[0], t109[0]), vbslq_u32(t111[1], t114[1], t109[1])];
                }
                x72 = [vbslq_u32(t102[0], t109[0], x72[0]), vbslq_u32(t102[1], t109[1], x72[1])];
            }
            x48 = [vbslq_u32(t86[0], x72[0], x48[0]), vbslq_u32(t86[1], x72[1], x48[1])];
            let t115: [uint32x4_t; 2] = [vandq_u32(t86[0], live[0]), vandq_u32(t86[1], live[1])];
            let t116 = [vdupq_n_u32(1u32); 2];
            let t117: [uint32x4_t; 2] = [vaddq_u32(x56[0], t116[0]), vaddq_u32(x56[1], t116[1])];
            let t118 = x56;
            x56 = t117;
        }
        let t119 = [vdupq_n_u32(2147483648u32); 2];
        let t120: [uint32x4_t; 2] = [vandq_u32(x48[0], t119[0]), vandq_u32(x48[1], t119[1])];
        let t121: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t120[0], t76[0])), vmvnq_u32(vceqq_u32(t120[1], t76[1]))];
        let t122 = [vdupq_n_u32(4294967294u32); 2];
        let t123: [uint32x4_t; 2] = [vcltq_u32(x48[0], t122[0]), vcltq_u32(x48[1], t122[1])];
        let t124: [uint32x4_t; 2] = [vandq_u32(t121[0], t123[0]), vandq_u32(t121[1], t123[1])];
        if any(&t124) {
            let t125 = [vdupq_n_u32((2147483648u32).wrapping_sub(1u32)); 2];
            let t126: [uint32x4_t; 2] = [vandq_u32(x48[0], t125[0]), vandq_u32(x48[1], t125[1])];
            let t127 = b5_r0r1l_p(l, lid0, t124, arg0, arg1, t126);
            x48 = [vbslq_u32(t124[0], t127[0], x48[0]), vbslq_u32(t124[1], t127[1], x48[1])];
        }
        b3_ag4r2l_f_v2(l, lid0, arg2, x48);
    } else if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((x24) as u64)) {
        return;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t128: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t129 = [vdupq_n_u32(x24); 2];
        let t130: [uint32x4_t; 2] = [vcgeq_u32(t128[0], t129[0]), vcgeq_u32(t128[1], t129[1])];
        let t131: [uint32x4_t; 2] = [vandq_u32(live[0], t130[0]), vandq_u32(live[1], t130[1])];
        if any(&t131) {
            live = [vbicq_u32(live[0], t131[0]), vbicq_u32(live[1], t131[1])];
        }
        if any(&live) {
            let t132 = b2_r0r1g_p(l, lid0, live, arg0, arg1);
            x40 = t132;
            let t133 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
            let t134: [uint64x2_t; 4] = [vandq_u64(x40[0], t133[0]), vandq_u64(x40[1], t133[1]), vandq_u64(x40[2], t133[2]), vandq_u64(x40[3], t133[3])];
            let t135 = [vdupq_n_u64(0u64); 4];
            let t136: [uint64x2_t; 4] = [vceqq_u64(t134[0], t135[0]), vceqq_u64(t134[1], t135[1]), vceqq_u64(t134[2], t135[2]), vceqq_u64(t134[3], t135[3])];
            let t137: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t136[0]), vmovn_u64(t136[1])), vcombine_u32(vmovn_u64(t136[2]), vmovn_u64(t136[3]))];
            let t138: [uint32x4_t; 2] = [vmvnq_u32(t137[0]), vmvnq_u32(t137[1])];
            let mut t139: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            let t140: [uint32x4_t; 2] = [vandq_u32(live[0], t138[0]), vandq_u32(live[1], t138[1])];
            let t141: [uint32x4_t; 2] = [vbicq_u32(live[0], t138[0]), vbicq_u32(live[1], t138[1])];
            {
                let t142: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x40[0]), vmovn_u64(x40[1])), vcombine_u32(vmovn_u64(x40[2]), vmovn_u64(x40[3]))];
                let t143 = [vdupq_n_u32(2147483648u32); 2];
                let t144: [uint32x4_t; 2] = [vorrq_u32(t143[0], t142[0]), vorrq_u32(t143[1], t142[1])];
                t139 = [vbslq_u32(t140[0], t144[0], t139[0]), vbslq_u32(t140[1], t144[1], t139[1])];
            }
            {
                let t145: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x40[0]), vmovn_u64(x40[1])), vcombine_u32(vmovn_u64(x40[2]), vmovn_u64(x40[3]))];
                t139 = [vbslq_u32(t141[0], t145[0], t139[0]), vbslq_u32(t141[1], t145[1], t139[1])];
            }
            x48 = t139;
            b3_ag4r2l_p(l, lid0, live, arg2, x48);
            let t146 = [vdupq_n_u32(0u32); 2];
            x56 = t146;
            let mut t147 = live;
            loop {
                // Lanes part: each its own rounds.
                let t149 = [vdupq_n_u32(7u32); 2];
                let t150: [uint32x4_t; 2] = [vcltq_u32(x56[0], t149[0]), vcltq_u32(x56[1], t149[1])];
                let t151 = [vdupq_n_u32(2147483648u32); 2];
                let t152: [uint32x4_t; 2] = [vandq_u32(x48[0], t151[0]), vandq_u32(x48[1], t151[1])];
                let t153: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t152[0], t146[0])), vmvnq_u32(vceqq_u32(t152[1], t146[1]))];
                let t154 = [vdupq_n_u32(4294967294u32); 2];
                let t155: [uint32x4_t; 2] = [vcltq_u32(x48[0], t154[0]), vcltq_u32(x48[1], t154[1])];
                let t156: [uint32x4_t; 2] = [vandq_u32(t153[0], t155[0]), vandq_u32(t153[1], t155[1])];
                let t157: [uint32x4_t; 2] = [vandq_u32(t150[0], t156[0]), vandq_u32(t150[1], t156[1])];
                t147 = [vandq_u32(t147[0], t157[0]), vandq_u32(t147[1], t157[1])];
                if !(any(&t147)) { break; }
                let t159 = [vdupq_n_u32((2147483648u32).wrapping_sub(1u32)); 2];
                let t160: [uint32x4_t; 2] = [vandq_u32(x48[0], t159[0]), vandq_u32(x48[1], t159[1])];
                x64 = t160;
                let t161 = b4_al4r2_p(l, lid0, t147, arg2, x64);
                x72 = t161;
                let t162: [uint32x4_t; 2] = [vceqq_u32(x72[0], t154[0]), vceqq_u32(x72[1], t154[1])];
                let t163: [uint32x4_t; 2] = [vandq_u32(t147[0], t162[0]), vandq_u32(t147[1], t162[1])];
                if any(&t163) {
                    let t164 = b2_r0r1l_p(l, lid0, t163, arg0, arg1, x64);
                    x80 = t164;
                    let t165 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                    let t166: [uint64x2_t; 4] = [vandq_u64(x80[0], t165[0]), vandq_u64(x80[1], t165[1]), vandq_u64(x80[2], t165[2]), vandq_u64(x80[3], t165[3])];
                    let t167: [uint64x2_t; 4] = [vceqq_u64(t166[0], t135[0]), vceqq_u64(t166[1], t135[1]), vceqq_u64(t166[2], t135[2]), vceqq_u64(t166[3], t135[3])];
                    let t168: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t167[0]), vmovn_u64(t167[1])), vcombine_u32(vmovn_u64(t167[2]), vmovn_u64(t167[3]))];
                    let t169: [uint32x4_t; 2] = [vmvnq_u32(t168[0]), vmvnq_u32(t168[1])];
                    let mut t170: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                    let t171: [uint32x4_t; 2] = [vandq_u32(t163[0], t169[0]), vandq_u32(t163[1], t169[1])];
                    let t172: [uint32x4_t; 2] = [vbicq_u32(t163[0], t169[0]), vbicq_u32(t163[1], t169[1])];
                    {
                        let t173: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x80[0]), vmovn_u64(x80[1])), vcombine_u32(vmovn_u64(x80[2]), vmovn_u64(x80[3]))];
                        let t174: [uint32x4_t; 2] = [vorrq_u32(t151[0], t173[0]), vorrq_u32(t151[1], t173[1])];
                        t170 = [vbslq_u32(t171[0], t174[0], t170[0]), vbslq_u32(t171[1], t174[1], t170[1])];
                    }
                    {
                        let t175: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x80[0]), vmovn_u64(x80[1])), vcombine_u32(vmovn_u64(x80[2]), vmovn_u64(x80[3]))];
                        t170 = [vbslq_u32(t172[0], t175[0], t170[0]), vbslq_u32(t172[1], t175[1], t170[1])];
                    }
                    x72 = [vbslq_u32(t163[0], t170[0], x72[0]), vbslq_u32(t163[1], t170[1], x72[1])];
                }
                x48 = [vbslq_u32(t147[0], x72[0], x48[0]), vbslq_u32(t147[1], x72[1], x48[1])];
                let t176: [uint32x4_t; 2] = [vandq_u32(t147[0], live[0]), vandq_u32(t147[1], live[1])];
                let t177 = [vdupq_n_u32(1u32); 2];
                let t178: [uint32x4_t; 2] = [vaddq_u32(x56[0], t177[0]), vaddq_u32(x56[1], t177[1])];
                let t179 = x56;
                x56 = t178;
            }
            let t180 = [vdupq_n_u32(2147483648u32); 2];
            let t181: [uint32x4_t; 2] = [vandq_u32(x48[0], t180[0]), vandq_u32(x48[1], t180[1])];
            let t182: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(t181[0], t146[0])), vmvnq_u32(vceqq_u32(t181[1], t146[1]))];
            let t183 = [vdupq_n_u32(4294967294u32); 2];
            let t184: [uint32x4_t; 2] = [vcltq_u32(x48[0], t183[0]), vcltq_u32(x48[1], t183[1])];
            let t185: [uint32x4_t; 2] = [vandq_u32(t182[0], t184[0]), vandq_u32(t182[1], t184[1])];
            let t186: [uint32x4_t; 2] = [vandq_u32(live[0], t185[0]), vandq_u32(live[1], t185[1])];
            if any(&t186) {
                let t187 = [vdupq_n_u32((2147483648u32).wrapping_sub(1u32)); 2];
                let t188: [uint32x4_t; 2] = [vandq_u32(x48[0], t187[0]), vandq_u32(x48[1], t187[1])];
                let t189 = b5_r0r1l_p(l, lid0, t186, arg0, arg1, t188);
                x48 = [vbslq_u32(t186[0], t189[0], x48[0]), vbslq_u32(t186[1], t189[1], x48[1])];
            }
            b3_ag4r2l_p(l, lid0, live, arg2, x48);
        }
    }
}

#[inline(always)]
unsafe fn b2_r0r1g_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t190: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t191 = [vdupq_n_u32(0u32); 2];
    let t192: [uint32x4_t; 2] = [vceqq_u32(t190[0], t191[0]), vceqq_u32(t190[1], t191[1])];
    let t193: [uint32x4_t; 2] = [vandq_u32(live[0], t192[0]), vandq_u32(live[1], t192[1])];
    if any(&t193) {
        let t194 = [vdupq_n_u64(4294967295u64); 4];
        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t193[0])))), t194[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t193[0])))), t194[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t193[1])))), t194[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t193[1])))), t194[3], res[3])];
        live = [vbicq_u32(live[0], t193[0]), vbicq_u32(live[1], t193[1])];
    }
    if any(&live) {
        let t195 = [vdupq_n_u32(1u32); 2];
        let t196: [uint32x4_t; 2] = [vsubq_u32(t190[0], t195[0]), vsubq_u32(t190[1], t195[1])];
        x24 = [vbslq_u32(live[0], t196[0], x24[0]), vbslq_u32(live[1], t196[1], x24[1])];
        let t197 = b6_r0r1l_p(l, lid0, live, arg0, arg1, x24);
        x32 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t197[0], x32[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t197[1], x32[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t197[2], x32[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t197[3], x32[3])];
        let t198 = [vdupq_n_u64(1u64); 4];
        let t199: [uint64x2_t; 4] = [vandq_u64(x32[0], t198[0]), vandq_u64(x32[1], t198[1]), vandq_u64(x32[2], t198[2]), vandq_u64(x32[3], t198[3])];
        let t200 = [vdupq_n_u64(0u64); 4];
        let t201: [uint64x2_t; 4] = [vceqq_u64(t199[0], t200[0]), vceqq_u64(t199[1], t200[1]), vceqq_u64(t199[2], t200[2]), vceqq_u64(t199[3], t200[3])];
        let t202: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t201[0]), vmovn_u64(t201[1])), vcombine_u32(vmovn_u64(t201[2]), vmovn_u64(t201[3]))];
        let t203: [uint32x4_t; 2] = [vandq_u32(live[0], t202[0]), vandq_u32(live[1], t202[1])];
        if any(&t203) {
            let t204: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x24[0])), vmovl_u32(vget_high_u32(x24[0])), vmovl_u32(vget_low_u32(x24[1])), vmovl_u32(vget_high_u32(x24[1]))];
            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t203[0])))), t204[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t203[0])))), t204[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t203[1])))), t204[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t203[1])))), t204[3], res[3])];
            live = [vbicq_u32(live[0], t203[0]), vbicq_u32(live[1], t203[1])];
        }
        if any(&live) {
            let t205: [uint64x2_t; 4] = [vshlq_u64(x32[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
            let t206: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t205[0]), vmovn_u64(t205[1])), vcombine_u32(vmovn_u64(t205[2]), vmovn_u64(t205[3]))];
            x40 = [vbslq_u32(live[0], t206[0], x40[0]), vbslq_u32(live[1], t206[1], x40[1])];
            let t207: [uint32x4_t; 2] = [vsubq_u32(x40[0], t195[0]), vsubq_u32(x40[1], t195[1])];
            let t208: [uint32x4_t; 2] = [vcgeq_u32(t207[0], t190[0]), vcgeq_u32(t207[1], t190[1])];
            let t209: [uint32x4_t; 2] = [vandq_u32(live[0], t208[0]), vandq_u32(live[1], t208[1])];
            if any(&t209) {
                let t210 = [vdupq_n_u64(4294967295u64); 4];
                res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t209[0])))), t210[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t209[0])))), t210[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t209[1])))), t210[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t209[1])))), t210[3], res[3])];
                live = [vbicq_u32(live[0], t209[0]), vbicq_u32(live[1], t209[1])];
            }
            if any(&live) {
                let t211: [uint32x4_t; 2] = [vsubq_u32(t190[0], x40[0]), vsubq_u32(t190[1], x40[1])];
                x48 = [vbslq_u32(live[0], t211[0], x48[0]), vbslq_u32(live[1], t211[1], x48[1])];
                let t212 = [vdupq_n_u64(2u64); 4];
                let t213: [uint64x2_t; 4] = [vandq_u64(x32[0], t212[0]), vandq_u64(x32[1], t212[1]), vandq_u64(x32[2], t212[2]), vandq_u64(x32[3], t212[3])];
                let t214: [uint64x2_t; 4] = [vceqq_u64(t213[0], t200[0]), vceqq_u64(t213[1], t200[1]), vceqq_u64(t213[2], t200[2]), vceqq_u64(t213[3], t200[3])];
                let t215: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t214[0]), vmovn_u64(t214[1])), vcombine_u32(vmovn_u64(t214[2]), vmovn_u64(t214[3]))];
                let t216: [uint32x4_t; 2] = [vmvnq_u32(t215[0]), vmvnq_u32(t215[1])];
                let t217: [uint32x4_t; 2] = [vandq_u32(live[0], t216[0]), vandq_u32(live[1], t216[1])];
                if any(&t217) {
                    let t218: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                    res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t217[0])))), t218[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t217[0])))), t218[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t217[1])))), t218[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t217[1])))), t218[3], res[3])];
                    live = [vbicq_u32(live[0], t217[0]), vbicq_u32(live[1], t217[1])];
                }
                if any(&live) {
                    let t219: [uint32x4_t; 2] = [vceqq_u32(x48[0], t191[0]), vceqq_u32(x48[1], t191[1])];
                    let t220: [uint32x4_t; 2] = [vandq_u32(live[0], t219[0]), vandq_u32(live[1], t219[1])];
                    if any(&t220) {
                        let t221 = [vdupq_n_u64(4294967295u64); 4];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t220[0])))), t221[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t220[0])))), t221[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t220[1])))), t221[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t220[1])))), t221[3], res[3])];
                        live = [vbicq_u32(live[0], t220[0]), vbicq_u32(live[1], t220[1])];
                    }
                    if any(&live) {
                        let t222 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                        let t223: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                        let t224: [uint64x2_t; 4] = [vorrq_u64(t222[0], t223[0]), vorrq_u64(t222[1], t223[1]), vorrq_u64(t222[2], t223[2]), vorrq_u64(t222[3], t223[3])];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t224[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t224[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t224[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t224[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    }
    res
}

#[inline(always)]
unsafe fn b2_r0r1g_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    if (((gid0) as u64 + 7) <= u32::MAX as u64 && (((0u32) as u64) < ((gid0) as u64) || ((0u32) as u64) > ((gid0) as u64 + 7))) {
        let t225: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t226 = [vdupq_n_u32(1u32); 2];
        let t227: [uint32x4_t; 2] = [vsubq_u32(t225[0], t226[0]), vsubq_u32(t225[1], t226[1])];
        x24 = t227;
        let t228 = b6_r0r1l_f(l, lid0, arg0, arg1, x24);
        x32 = t228;
        let t229 = [vdupq_n_u64(1u64); 4];
        let t230: [uint64x2_t; 4] = [vandq_u64(x32[0], t229[0]), vandq_u64(x32[1], t229[1]), vandq_u64(x32[2], t229[2]), vandq_u64(x32[3], t229[3])];
        let t231 = [vdupq_n_u64(0u64); 4];
        let t232: [uint64x2_t; 4] = [vceqq_u64(t230[0], t231[0]), vceqq_u64(t230[1], t231[1]), vceqq_u64(t230[2], t231[2]), vceqq_u64(t230[3], t231[3])];
        let t233: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t232[0]), vmovn_u64(t232[1])), vcombine_u32(vmovn_u64(t232[2]), vmovn_u64(t232[3]))];
        if any(&t233) {
            let t234: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x24[0])), vmovl_u32(vget_high_u32(x24[0])), vmovl_u32(vget_low_u32(x24[1])), vmovl_u32(vget_high_u32(x24[1]))];
            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t233[0])))), t234[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t233[0])))), t234[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t233[1])))), t234[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t233[1])))), t234[3], res[3])];
            live = [vbicq_u32(live[0], t233[0]), vbicq_u32(live[1], t233[1])];
        }
        if any(&live) {
            let t235: [uint64x2_t; 4] = [vshlq_u64(x32[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
            let t236: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t235[0]), vmovn_u64(t235[1])), vcombine_u32(vmovn_u64(t235[2]), vmovn_u64(t235[3]))];
            x40 = [vbslq_u32(live[0], t236[0], x40[0]), vbslq_u32(live[1], t236[1], x40[1])];
            let t237: [uint32x4_t; 2] = [vsubq_u32(x40[0], t226[0]), vsubq_u32(x40[1], t226[1])];
            let t238: [uint32x4_t; 2] = [vcgeq_u32(t237[0], t225[0]), vcgeq_u32(t237[1], t225[1])];
            let t239: [uint32x4_t; 2] = [vandq_u32(live[0], t238[0]), vandq_u32(live[1], t238[1])];
            if any(&t239) {
                let t240 = [vdupq_n_u64(4294967295u64); 4];
                res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t239[0])))), t240[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t239[0])))), t240[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t239[1])))), t240[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t239[1])))), t240[3], res[3])];
                live = [vbicq_u32(live[0], t239[0]), vbicq_u32(live[1], t239[1])];
            }
            if any(&live) {
                let t241: [uint32x4_t; 2] = [vsubq_u32(t225[0], x40[0]), vsubq_u32(t225[1], x40[1])];
                x48 = [vbslq_u32(live[0], t241[0], x48[0]), vbslq_u32(live[1], t241[1], x48[1])];
                let t242 = [vdupq_n_u64(2u64); 4];
                let t243: [uint64x2_t; 4] = [vandq_u64(x32[0], t242[0]), vandq_u64(x32[1], t242[1]), vandq_u64(x32[2], t242[2]), vandq_u64(x32[3], t242[3])];
                let t244: [uint64x2_t; 4] = [vceqq_u64(t243[0], t231[0]), vceqq_u64(t243[1], t231[1]), vceqq_u64(t243[2], t231[2]), vceqq_u64(t243[3], t231[3])];
                let t245: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t244[0]), vmovn_u64(t244[1])), vcombine_u32(vmovn_u64(t244[2]), vmovn_u64(t244[3]))];
                let t246: [uint32x4_t; 2] = [vmvnq_u32(t245[0]), vmvnq_u32(t245[1])];
                let t247: [uint32x4_t; 2] = [vandq_u32(live[0], t246[0]), vandq_u32(live[1], t246[1])];
                if any(&t247) {
                    let t248: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                    res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t247[0])))), t248[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t247[0])))), t248[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t247[1])))), t248[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t247[1])))), t248[3], res[3])];
                    live = [vbicq_u32(live[0], t247[0]), vbicq_u32(live[1], t247[1])];
                }
                if any(&live) {
                    let t249 = [vdupq_n_u32(0u32); 2];
                    let t250: [uint32x4_t; 2] = [vceqq_u32(x48[0], t249[0]), vceqq_u32(x48[1], t249[1])];
                    let t251: [uint32x4_t; 2] = [vandq_u32(live[0], t250[0]), vandq_u32(live[1], t250[1])];
                    if any(&t251) {
                        let t252 = [vdupq_n_u64(4294967295u64); 4];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t251[0])))), t252[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t251[0])))), t252[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t251[1])))), t252[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t251[1])))), t252[3], res[3])];
                        live = [vbicq_u32(live[0], t251[0]), vbicq_u32(live[1], t251[1])];
                    }
                    if any(&live) {
                        let t253 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                        let t254: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                        let t255: [uint64x2_t; 4] = [vorrq_u64(t253[0], t254[0]), vorrq_u64(t253[1], t254[1]), vorrq_u64(t253[2], t254[2]), vorrq_u64(t253[3], t254[3])];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t255[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t255[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t255[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t255[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    } else if false {
        let t256 = [vdupq_n_u64(4294967295u64); 4];
        res = t256;
        return res;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t257: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t258 = [vdupq_n_u32(0u32); 2];
        let t259: [uint32x4_t; 2] = [vceqq_u32(t257[0], t258[0]), vceqq_u32(t257[1], t258[1])];
        let t260: [uint32x4_t; 2] = [vandq_u32(live[0], t259[0]), vandq_u32(live[1], t259[1])];
        if any(&t260) {
            let t261 = [vdupq_n_u64(4294967295u64); 4];
            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t260[0])))), t261[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t260[0])))), t261[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t260[1])))), t261[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t260[1])))), t261[3], res[3])];
            live = [vbicq_u32(live[0], t260[0]), vbicq_u32(live[1], t260[1])];
        }
        if any(&live) {
            let t262 = [vdupq_n_u32(1u32); 2];
            let t263: [uint32x4_t; 2] = [vsubq_u32(t257[0], t262[0]), vsubq_u32(t257[1], t262[1])];
            x24 = [vbslq_u32(live[0], t263[0], x24[0]), vbslq_u32(live[1], t263[1], x24[1])];
            let t264 = b6_r0r1l_p(l, lid0, live, arg0, arg1, x24);
            x32 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t264[0], x32[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t264[1], x32[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t264[2], x32[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t264[3], x32[3])];
            let t265 = [vdupq_n_u64(1u64); 4];
            let t266: [uint64x2_t; 4] = [vandq_u64(x32[0], t265[0]), vandq_u64(x32[1], t265[1]), vandq_u64(x32[2], t265[2]), vandq_u64(x32[3], t265[3])];
            let t267 = [vdupq_n_u64(0u64); 4];
            let t268: [uint64x2_t; 4] = [vceqq_u64(t266[0], t267[0]), vceqq_u64(t266[1], t267[1]), vceqq_u64(t266[2], t267[2]), vceqq_u64(t266[3], t267[3])];
            let t269: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t268[0]), vmovn_u64(t268[1])), vcombine_u32(vmovn_u64(t268[2]), vmovn_u64(t268[3]))];
            let t270: [uint32x4_t; 2] = [vandq_u32(live[0], t269[0]), vandq_u32(live[1], t269[1])];
            if any(&t270) {
                let t271: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x24[0])), vmovl_u32(vget_high_u32(x24[0])), vmovl_u32(vget_low_u32(x24[1])), vmovl_u32(vget_high_u32(x24[1]))];
                res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t270[0])))), t271[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t270[0])))), t271[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t270[1])))), t271[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t270[1])))), t271[3], res[3])];
                live = [vbicq_u32(live[0], t270[0]), vbicq_u32(live[1], t270[1])];
            }
            if any(&live) {
                let t272: [uint64x2_t; 4] = [vshlq_u64(x32[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
                let t273: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t272[0]), vmovn_u64(t272[1])), vcombine_u32(vmovn_u64(t272[2]), vmovn_u64(t272[3]))];
                x40 = [vbslq_u32(live[0], t273[0], x40[0]), vbslq_u32(live[1], t273[1], x40[1])];
                let t274: [uint32x4_t; 2] = [vsubq_u32(x40[0], t262[0]), vsubq_u32(x40[1], t262[1])];
                let t275: [uint32x4_t; 2] = [vcgeq_u32(t274[0], t257[0]), vcgeq_u32(t274[1], t257[1])];
                let t276: [uint32x4_t; 2] = [vandq_u32(live[0], t275[0]), vandq_u32(live[1], t275[1])];
                if any(&t276) {
                    let t277 = [vdupq_n_u64(4294967295u64); 4];
                    res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t276[0])))), t277[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t276[0])))), t277[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t276[1])))), t277[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t276[1])))), t277[3], res[3])];
                    live = [vbicq_u32(live[0], t276[0]), vbicq_u32(live[1], t276[1])];
                }
                if any(&live) {
                    let t278: [uint32x4_t; 2] = [vsubq_u32(t257[0], x40[0]), vsubq_u32(t257[1], x40[1])];
                    x48 = [vbslq_u32(live[0], t278[0], x48[0]), vbslq_u32(live[1], t278[1], x48[1])];
                    let t279 = [vdupq_n_u64(2u64); 4];
                    let t280: [uint64x2_t; 4] = [vandq_u64(x32[0], t279[0]), vandq_u64(x32[1], t279[1]), vandq_u64(x32[2], t279[2]), vandq_u64(x32[3], t279[3])];
                    let t281: [uint64x2_t; 4] = [vceqq_u64(t280[0], t267[0]), vceqq_u64(t280[1], t267[1]), vceqq_u64(t280[2], t267[2]), vceqq_u64(t280[3], t267[3])];
                    let t282: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t281[0]), vmovn_u64(t281[1])), vcombine_u32(vmovn_u64(t281[2]), vmovn_u64(t281[3]))];
                    let t283: [uint32x4_t; 2] = [vmvnq_u32(t282[0]), vmvnq_u32(t282[1])];
                    let t284: [uint32x4_t; 2] = [vandq_u32(live[0], t283[0]), vandq_u32(live[1], t283[1])];
                    if any(&t284) {
                        let t285: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t284[0])))), t285[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t284[0])))), t285[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t284[1])))), t285[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t284[1])))), t285[3], res[3])];
                        live = [vbicq_u32(live[0], t284[0]), vbicq_u32(live[1], t284[1])];
                    }
                    if any(&live) {
                        let t286: [uint32x4_t; 2] = [vceqq_u32(x48[0], t258[0]), vceqq_u32(x48[1], t258[1])];
                        let t287: [uint32x4_t; 2] = [vandq_u32(live[0], t286[0]), vandq_u32(live[1], t286[1])];
                        if any(&t287) {
                            let t288 = [vdupq_n_u64(4294967295u64); 4];
                            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t287[0])))), t288[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t287[0])))), t288[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t287[1])))), t288[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t287[1])))), t288[3], res[3])];
                            live = [vbicq_u32(live[0], t287[0]), vbicq_u32(live[1], t287[1])];
                        }
                        if any(&live) {
                            let t289 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                            let t290: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                            let t291: [uint64x2_t; 4] = [vorrq_u64(t289[0], t290[0]), vorrq_u64(t289[1], t290[1]), vorrq_u64(t289[2], t290[2]), vorrq_u64(t289[3], t290[3])];
                            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t291[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t291[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t291[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t291[3], res[3])];
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
unsafe fn b2_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t292 = [vdupq_n_u32(0u32); 2];
    let t293: [uint32x4_t; 2] = [vceqq_u32(x16[0], t292[0]), vceqq_u32(x16[1], t292[1])];
    let t294: [uint32x4_t; 2] = [vandq_u32(live[0], t293[0]), vandq_u32(live[1], t293[1])];
    if any(&t294) {
        let t295 = [vdupq_n_u64(4294967295u64); 4];
        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t294[0])))), t295[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t294[0])))), t295[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t294[1])))), t295[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t294[1])))), t295[3], res[3])];
        live = [vbicq_u32(live[0], t294[0]), vbicq_u32(live[1], t294[1])];
    }
    if any(&live) {
        let t296 = [vdupq_n_u32(1u32); 2];
        let t297: [uint32x4_t; 2] = [vsubq_u32(x16[0], t296[0]), vsubq_u32(x16[1], t296[1])];
        x24 = [vbslq_u32(live[0], t297[0], x24[0]), vbslq_u32(live[1], t297[1], x24[1])];
        let t298 = b6_r0r1l_p(l, lid0, live, arg0, arg1, x24);
        x32 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t298[0], x32[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t298[1], x32[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t298[2], x32[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t298[3], x32[3])];
        let t299 = [vdupq_n_u64(1u64); 4];
        let t300: [uint64x2_t; 4] = [vandq_u64(x32[0], t299[0]), vandq_u64(x32[1], t299[1]), vandq_u64(x32[2], t299[2]), vandq_u64(x32[3], t299[3])];
        let t301 = [vdupq_n_u64(0u64); 4];
        let t302: [uint64x2_t; 4] = [vceqq_u64(t300[0], t301[0]), vceqq_u64(t300[1], t301[1]), vceqq_u64(t300[2], t301[2]), vceqq_u64(t300[3], t301[3])];
        let t303: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t302[0]), vmovn_u64(t302[1])), vcombine_u32(vmovn_u64(t302[2]), vmovn_u64(t302[3]))];
        let t304: [uint32x4_t; 2] = [vandq_u32(live[0], t303[0]), vandq_u32(live[1], t303[1])];
        if any(&t304) {
            let t305: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x24[0])), vmovl_u32(vget_high_u32(x24[0])), vmovl_u32(vget_low_u32(x24[1])), vmovl_u32(vget_high_u32(x24[1]))];
            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t304[0])))), t305[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t304[0])))), t305[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t304[1])))), t305[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t304[1])))), t305[3], res[3])];
            live = [vbicq_u32(live[0], t304[0]), vbicq_u32(live[1], t304[1])];
        }
        if any(&live) {
            let t306: [uint64x2_t; 4] = [vshlq_u64(x32[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
            let t307: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t306[0]), vmovn_u64(t306[1])), vcombine_u32(vmovn_u64(t306[2]), vmovn_u64(t306[3]))];
            x40 = [vbslq_u32(live[0], t307[0], x40[0]), vbslq_u32(live[1], t307[1], x40[1])];
            let t308: [uint32x4_t; 2] = [vsubq_u32(x40[0], t296[0]), vsubq_u32(x40[1], t296[1])];
            let t309: [uint32x4_t; 2] = [vcgeq_u32(t308[0], x16[0]), vcgeq_u32(t308[1], x16[1])];
            let t310: [uint32x4_t; 2] = [vandq_u32(live[0], t309[0]), vandq_u32(live[1], t309[1])];
            if any(&t310) {
                let t311 = [vdupq_n_u64(4294967295u64); 4];
                res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t310[0])))), t311[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t310[0])))), t311[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t310[1])))), t311[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t310[1])))), t311[3], res[3])];
                live = [vbicq_u32(live[0], t310[0]), vbicq_u32(live[1], t310[1])];
            }
            if any(&live) {
                let t312: [uint32x4_t; 2] = [vsubq_u32(x16[0], x40[0]), vsubq_u32(x16[1], x40[1])];
                x48 = [vbslq_u32(live[0], t312[0], x48[0]), vbslq_u32(live[1], t312[1], x48[1])];
                let t313 = [vdupq_n_u64(2u64); 4];
                let t314: [uint64x2_t; 4] = [vandq_u64(x32[0], t313[0]), vandq_u64(x32[1], t313[1]), vandq_u64(x32[2], t313[2]), vandq_u64(x32[3], t313[3])];
                let t315: [uint64x2_t; 4] = [vceqq_u64(t314[0], t301[0]), vceqq_u64(t314[1], t301[1]), vceqq_u64(t314[2], t301[2]), vceqq_u64(t314[3], t301[3])];
                let t316: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t315[0]), vmovn_u64(t315[1])), vcombine_u32(vmovn_u64(t315[2]), vmovn_u64(t315[3]))];
                let t317: [uint32x4_t; 2] = [vmvnq_u32(t316[0]), vmvnq_u32(t316[1])];
                let t318: [uint32x4_t; 2] = [vandq_u32(live[0], t317[0]), vandq_u32(live[1], t317[1])];
                if any(&t318) {
                    let t319: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                    res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t318[0])))), t319[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t318[0])))), t319[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t318[1])))), t319[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t318[1])))), t319[3], res[3])];
                    live = [vbicq_u32(live[0], t318[0]), vbicq_u32(live[1], t318[1])];
                }
                if any(&live) {
                    let t320: [uint32x4_t; 2] = [vceqq_u32(x48[0], t292[0]), vceqq_u32(x48[1], t292[1])];
                    let t321: [uint32x4_t; 2] = [vandq_u32(live[0], t320[0]), vandq_u32(live[1], t320[1])];
                    if any(&t321) {
                        let t322 = [vdupq_n_u64(4294967295u64); 4];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t321[0])))), t322[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t321[0])))), t322[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t321[1])))), t322[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t321[1])))), t322[3], res[3])];
                        live = [vbicq_u32(live[0], t321[0]), vbicq_u32(live[1], t321[1])];
                    }
                    if any(&live) {
                        let t323 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                        let t324: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                        let t325: [uint64x2_t; 4] = [vorrq_u64(t323[0], t324[0]), vorrq_u64(t323[1], t324[1]), vorrq_u64(t323[2], t324[2]), vorrq_u64(t323[3], t324[3])];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t325[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t325[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t325[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t325[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    }
    res
}

#[inline(always)]
unsafe fn b3_ag4r2l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg2: *const u8, x8: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let t326 = (arg2 as *mut u32).add((gid0) as usize);
    if all(&live) {
        vst1q_u32(t326.add(0), x8[0]);
        vst1q_u32(t326.add(4), x8[1]);
    } else {
        let mut t327w = 0u32;
        let t327 = core::hint::black_box(&raw mut t327w);
        core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t326.add(0), t327).write(vgetq_lane_u32::<0>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t326.add(1), t327).write(vgetq_lane_u32::<1>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t326.add(2), t327).write(vgetq_lane_u32::<2>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t326.add(3), t327).write(vgetq_lane_u32::<3>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t326.add(4), t327).write(vgetq_lane_u32::<0>(x8[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t326.add(5), t327).write(vgetq_lane_u32::<1>(x8[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t326.add(6), t327).write(vgetq_lane_u32::<2>(x8[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t326.add(7), t327).write(vgetq_lane_u32::<3>(x8[1]));
    }
}

#[inline(always)]
unsafe fn b3_ag4r2l_f(l: &Launch, lid0: u32, arg2: *const u8, x8: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let t328 = (arg2 as *mut u32).add((gid0) as usize);
    vst1q_u32(t328.add(0), x8[0]);
    vst1q_u32(t328.add(4), x8[1]);
}

#[inline(always)]
unsafe fn b3_ag4r2l_f_v2(l: &Launch, lid0: u32, arg2: *const u8, x8: [uint32x4_t; 2]) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let t329 = (arg2 as *mut u32).add((gid0) as usize);
    vst1q_u32(t329.add(0), x8[0]);
    vst1q_u32(t329.add(4), x8[1]);
}

#[inline(always)]
unsafe fn b4_al4r2_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg2: *const u8, x0i: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t330 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x0i[0]), vandq_u32(live[1], x0i[1]))));
    let t331: [uint32x4_t; 2] = [vbslq_u32(live[0], x0i[0], t330), vbslq_u32(live[1], x0i[1], t330)];
    let t332 = arg2 as *const u32;
    let mut t333: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    t333[0] = vld1q_lane_u32::<0>(t332.add(vgetq_lane_u32::<0>(t331[0]) as usize), t333[0]);
    t333[0] = vld1q_lane_u32::<1>(t332.add(vgetq_lane_u32::<1>(t331[0]) as usize), t333[0]);
    t333[0] = vld1q_lane_u32::<2>(t332.add(vgetq_lane_u32::<2>(t331[0]) as usize), t333[0]);
    t333[0] = vld1q_lane_u32::<3>(t332.add(vgetq_lane_u32::<3>(t331[0]) as usize), t333[0]);
    t333[1] = vld1q_lane_u32::<0>(t332.add(vgetq_lane_u32::<0>(t331[1]) as usize), t333[1]);
    t333[1] = vld1q_lane_u32::<1>(t332.add(vgetq_lane_u32::<1>(t331[1]) as usize), t333[1]);
    t333[1] = vld1q_lane_u32::<2>(t332.add(vgetq_lane_u32::<2>(t331[1]) as usize), t333[1]);
    t333[1] = vld1q_lane_u32::<3>(t332.add(vgetq_lane_u32::<3>(t331[1]) as usize), t333[1]);
    res = t333;
    return res;
}

#[inline(always)]
unsafe fn b4_al4r2_p_v2(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg2: *const u8, x0i: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t334: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t335: [uint32x4_t; 2] = [vbslq_u32(live[0], x0i[0], t334[0]), vbslq_u32(live[1], x0i[1], t334[1])];
    let t336 = arg2 as *const u32;
    let mut t337: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    t337[0] = vld1q_lane_u32::<0>(t336.add(vgetq_lane_u32::<0>(t335[0]) as usize), t337[0]);
    t337[0] = vld1q_lane_u32::<1>(t336.add(vgetq_lane_u32::<1>(t335[0]) as usize), t337[0]);
    t337[0] = vld1q_lane_u32::<2>(t336.add(vgetq_lane_u32::<2>(t335[0]) as usize), t337[0]);
    t337[0] = vld1q_lane_u32::<3>(t336.add(vgetq_lane_u32::<3>(t335[0]) as usize), t337[0]);
    t337[1] = vld1q_lane_u32::<0>(t336.add(vgetq_lane_u32::<0>(t335[1]) as usize), t337[1]);
    t337[1] = vld1q_lane_u32::<1>(t336.add(vgetq_lane_u32::<1>(t335[1]) as usize), t337[1]);
    t337[1] = vld1q_lane_u32::<2>(t336.add(vgetq_lane_u32::<2>(t335[1]) as usize), t337[1]);
    t337[1] = vld1q_lane_u32::<3>(t336.add(vgetq_lane_u32::<3>(t335[1]) as usize), t337[1]);
    res = t337;
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t338 = b2_r0r1l_p(l, lid0, live, arg0, arg1, x16);
    x24 = t338;
    let mut t339 = live;
    loop {
        // Lanes part: each its own rounds.
        let t341 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
        let t342: [uint64x2_t; 4] = [vandq_u64(x24[0], t341[0]), vandq_u64(x24[1], t341[1]), vandq_u64(x24[2], t341[2]), vandq_u64(x24[3], t341[3])];
        let t343 = [vdupq_n_u64(0u64); 4];
        let t344: [uint64x2_t; 4] = [vceqq_u64(t342[0], t343[0]), vceqq_u64(t342[1], t343[1]), vceqq_u64(t342[2], t343[2]), vceqq_u64(t342[3], t343[3])];
        let t345: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t344[0]), vmovn_u64(t344[1])), vcombine_u32(vmovn_u64(t344[2]), vmovn_u64(t344[3]))];
        let t346: [uint32x4_t; 2] = [vmvnq_u32(t345[0]), vmvnq_u32(t345[1])];
        t339 = [vandq_u32(t339[0], t346[0]), vandq_u32(t339[1], t346[1])];
        if !(any(&t339)) { break; }
        let t348: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x24[0]), vmovn_u64(x24[1])), vcombine_u32(vmovn_u64(x24[2]), vmovn_u64(x24[3]))];
        let t349 = b2_r0r1l_p(l, lid0, t339, arg0, arg1, t348);
        x24 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t339[0])))), t349[0], x24[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t339[0])))), t349[1], x24[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t339[1])))), t349[2], x24[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t339[1])))), t349[3], x24[3])];
    }
    let t350: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x24[0]), vmovn_u64(x24[1])), vcombine_u32(vmovn_u64(x24[2]), vmovn_u64(x24[3]))];
    res = t350;
    return res;
}

#[inline(always)]
unsafe fn b6_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t351 = b7_r0r1sl_p(l, lid0, live, arg0, arg1, 0u32, x16);
    res = t351;
    return res;
}

#[inline(always)]
unsafe fn b6_r0r1l_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t352 = b7_r0r1sl_f(l, lid0, arg0, arg1, 0u32, x16);
    res = t352;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: u64 = 0;
    let mut x56: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t353 = b8_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    x32 = t353;
    let t354: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t355 = b9_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x16), t354);
    x40 = t355;
    let t356 = b10_s_p(l, lid0, live, x32);
    x48 = t356;
    let t357 = [vdupq_n_u32((1024u32).wrapping_sub(1u32)); 2];
    let t358: [uint32x4_t; 2] = [vandq_u32(x24[0], t357[0]), vandq_u32(x24[1], t357[1])];
    let t359 = [vdupq_n_u32(x32); 2];
    let t360: [uint32x4_t; 2] = [vmulq_u32(t358[0], t359[0]), vmulq_u32(t358[1], t359[1])];
    let t361 = b11_r0r1lls_p(l, lid0, live, arg0, arg1, x40, t360, 8u32);
    let t362 = [vdupq_n_u64(x48); 4];
    let t363: [uint64x2_t; 4] = [vandq_u64(t361[0], t362[0]), vandq_u64(t361[1], t362[1]), vandq_u64(t361[2], t362[2]), vandq_u64(t361[3], t362[3])];
    x56 = t363;
    let t364 = [vdupq_n_u64(x48); 4];
    let t365: [uint64x2_t; 4] = [vceqq_u64(x56[0], t364[0]), vceqq_u64(x56[1], t364[1]), vceqq_u64(x56[2], t364[2]), vceqq_u64(x56[3], t364[3])];
    let t366: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t365[0]), vmovn_u64(t365[1])), vcombine_u32(vmovn_u64(t365[2]), vmovn_u64(t365[3]))];
    let mut t367: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t368: [uint32x4_t; 2] = [vandq_u32(live[0], t366[0]), vandq_u32(live[1], t366[1])];
    let t369: [uint32x4_t; 2] = [vbicq_u32(live[0], t366[0]), vbicq_u32(live[1], t366[1])];
    if any(&t368) {
        let t370 = b12_r0r1sl_p(l, lid0, t368, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t367 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t368[0])))), t370[0], t367[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t368[0])))), t370[1], t367[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t368[1])))), t370[2], t367[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t368[1])))), t370[3], t367[3])];
    }
    {
        t367 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t369[0])))), x56[0], t367[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t369[0])))), x56[1], t367[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t369[1])))), x56[2], t367[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t369[1])))), x56[3], t367[3])];
    }
    res = t367;
    return res;
}

#[inline(always)]
unsafe fn b7_r0r1sl_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: u64 = 0;
    let mut x56: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t371 = b8_r0r1s_f(l, lid0, arg0, arg1, x16);
    x32 = t371;
    let t372: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t373 = b9_r0r1sl_f(l, lid0, arg0, arg1, (1u32).wrapping_add(x16), t372);
    x40 = t373;
    let t374 = b10_s_f(l, lid0, x32);
    x48 = t374;
    let t375 = [vdupq_n_u32((1024u32).wrapping_sub(1u32)); 2];
    let t376: [uint32x4_t; 2] = [vandq_u32(x24[0], t375[0]), vandq_u32(x24[1], t375[1])];
    let t377 = [vdupq_n_u32(x32); 2];
    let t378: [uint32x4_t; 2] = [vmulq_u32(t376[0], t377[0]), vmulq_u32(t376[1], t377[1])];
    let t379 = b11_r0r1lls_f(l, lid0, arg0, arg1, x40, t378, 8u32);
    let t380 = [vdupq_n_u64(x48); 4];
    let t381: [uint64x2_t; 4] = [vandq_u64(t379[0], t380[0]), vandq_u64(t379[1], t380[1]), vandq_u64(t379[2], t380[2]), vandq_u64(t379[3], t380[3])];
    x56 = t381;
    let t382 = [vdupq_n_u64(x48); 4];
    let t383: [uint64x2_t; 4] = [vceqq_u64(x56[0], t382[0]), vceqq_u64(x56[1], t382[1]), vceqq_u64(x56[2], t382[2]), vceqq_u64(x56[3], t382[3])];
    let t384: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t383[0]), vmovn_u64(t383[1])), vcombine_u32(vmovn_u64(t383[2]), vmovn_u64(t383[3]))];
    let mut t385: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t386: [uint32x4_t; 2] = [vmvnq_u32(t384[0]), vmvnq_u32(t384[1])];
    if any(&t384) {
        let t387 = b12_r0r1sl_p(l, lid0, t384, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t385 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t384[0])))), t387[0], t385[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t384[0])))), t387[1], t385[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t384[1])))), t387[2], t385[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t384[1])))), t387[3], t385[3])];
    }
    {
        t385 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t386[0])))), x56[0], t385[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t386[0])))), x56[1], t385[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t386[1])))), x56[2], t385[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t386[1])))), x56[3], t385[3])];
    }
    res = t385;
    return res;
}

#[inline(always)]
unsafe fn b8_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t388 = b13_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    res = (((t388).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b8_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t389 = b13_r0r1s_f(l, lid0, arg0, arg1, 16u32);
    res = (((t389).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t390 = b13_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t391 = [vdupq_n_u32(t390); 2];
    let t392: [uint32x4_t; 2] = [vmulq_u32(x24[0], t391[0]), vmulq_u32(x24[1], t391[1])];
    let t393 = b14_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    let t394 = [vdupq_n_u32(t393); 2];
    let t395: [uint32x4_t; 2] = [vaddq_u32(t392[0], t394[0]), vaddq_u32(t392[1], t394[1])];
    x32 = t395;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t396 = b13_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t397: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t398 = [vdupq_n_u32(t396); 2];
    let t399: [uint32x4_t; 2] = [vsubq_u32(t398[0], t397[0]), vsubq_u32(t398[1], t397[1])];
    let t400 = b15_ls_p(l, lid0, live, t399, 1024u32);
    x40 = t400;
    let t401 = b14_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    let t402 = [vdupq_n_u32(t401); 2];
    let t403: [uint32x4_t; 2] = [vmulq_u32(x40[0], t402[0]), vmulq_u32(x40[1], t402[1])];
    let t404: [uint32x4_t; 2] = [vaddq_u32(x32[0], t403[0]), vaddq_u32(x32[1], t403[1])];
    res = t404;
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1sl_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t405 = b13_r0r1s_f(l, lid0, arg0, arg1, 40u32);
    let t406 = [vdupq_n_u32(t405); 2];
    let t407: [uint32x4_t; 2] = [vmulq_u32(x24[0], t406[0]), vmulq_u32(x24[1], t406[1])];
    let t408 = b14_r0r1s_f(l, lid0, arg0, arg1, 0u32);
    let t409 = [vdupq_n_u32(t408); 2];
    let t410: [uint32x4_t; 2] = [vaddq_u32(t407[0], t409[0]), vaddq_u32(t407[1], t409[1])];
    x32 = t410;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t411 = b13_r0r1s_f(l, lid0, arg0, arg1, 4u32);
    let t412: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t413 = [vdupq_n_u32(t411); 2];
    let t414: [uint32x4_t; 2] = [vsubq_u32(t413[0], t412[0]), vsubq_u32(t413[1], t412[1])];
    let t415 = b15_ls_f(l, lid0, t414, 1024u32);
    x40 = t415;
    let t416 = b14_r0r1s_f(l, lid0, arg0, arg1, x16);
    let t417 = [vdupq_n_u32(t416); 2];
    let t418: [uint32x4_t; 2] = [vmulq_u32(x40[0], t417[0]), vmulq_u32(x40[1], t417[1])];
    let t419: [uint32x4_t; 2] = [vaddq_u32(x32[0], t418[0]), vaddq_u32(x32[1], t418[1])];
    res = t419;
    return res;
}

#[inline(always)]
unsafe fn b10_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t420: u64;
    if ((x0) >= (8u32)) {
        t420 = !(0u64);
    } else {
        t420 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t420;
    return res;
}

#[inline(always)]
unsafe fn b10_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t421: u64;
    if ((x0) >= (8u32)) {
        t421 = !(0u64);
    } else {
        t421 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t421;
    return res;
}

#[inline(always)]
unsafe fn b10_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t422 = [vdupq_n_u32(8u32); 2];
    let t423: [uint32x4_t; 2] = [vcgeq_u32(x0[0], t422[0]), vcgeq_u32(x0[1], t422[1])];
    let mut t424: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t425: [uint32x4_t; 2] = [vandq_u32(live[0], t423[0]), vandq_u32(live[1], t423[1])];
    let t426: [uint32x4_t; 2] = [vbicq_u32(live[0], t423[0]), vbicq_u32(live[1], t423[1])];
    {
        let t427 = [vdupq_n_u64(!(0u64)); 4];
        t424 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t425[0])))), t427[0], t424[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t425[0])))), t427[1], t424[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t425[1])))), t427[2], t424[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t425[1])))), t427[3], t424[3])];
    }
    {
        let t428: [uint32x4_t; 2] = [vmulq_u32(x0[0], t422[0]), vmulq_u32(x0[1], t422[1])];
        let t429 = [vdupq_n_u64(1u64); 4];
        let t430: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t428[0])), vmovl_u32(vget_high_u32(t428[0])), vmovl_u32(vget_low_u32(t428[1])), vmovl_u32(vget_high_u32(t428[1]))];
        let t431: [uint64x2_t; 4] = [vshlq_u64(t429[0], vreinterpretq_s64_u64(vandq_u64(t430[0], vdupq_n_u64(63)))), vshlq_u64(t429[1], vreinterpretq_s64_u64(vandq_u64(t430[1], vdupq_n_u64(63)))), vshlq_u64(t429[2], vreinterpretq_s64_u64(vandq_u64(t430[2], vdupq_n_u64(63)))), vshlq_u64(t429[3], vreinterpretq_s64_u64(vandq_u64(t430[3], vdupq_n_u64(63))))];
        let t432: [uint64x2_t; 4] = [vsubq_u64(t431[0], t429[0]), vsubq_u64(t431[1], t429[1]), vsubq_u64(t431[2], t429[2]), vsubq_u64(t431[3], t429[3])];
        t424 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t426[0])))), t432[0], t424[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t426[0])))), t432[1], t424[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t426[1])))), t432[2], t424[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t426[1])))), t432[3], t424[3])];
    }
    res = t424;
    return res;
}

#[inline(always)]
unsafe fn b11_r0r1lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t433 = b16_r0lls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t433;
    return res;
}

#[inline(always)]
unsafe fn b11_r0r1lls_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t434 = b16_r0lls_f(l, lid0, arg0, x16, x24, x32);
    res = t434;
    return res;
}

#[inline(always)]
unsafe fn b12_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
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
    let mut x136: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x144: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x152: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x160: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x168: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x176: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x184: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t435 = b13_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t435;
    let t436 = b13_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t436;
    let t437: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x48 = t437;
    let t438 = [vdupq_n_u32(4u32); 2];
    let t439: [uint32x4_t; 2] = [vmulq_u32(x48[0], t438[0]), vmulq_u32(x48[1], t438[1])];
    let t440 = b17_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t439, 4u32);
    let t441: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t440[0]), vmovn_u64(t440[1])), vcombine_u32(vmovn_u64(t440[2]), vmovn_u64(t440[3]))];
    x56 = t441;
    let t442: [uint32x4_t; 2] = [vmulq_u32(x48[0], t438[0]), vmulq_u32(x48[1], t438[1])];
    let t443: [uint32x4_t; 2] = [vaddq_u32(t442[0], t438[0]), vaddq_u32(t442[1], t438[1])];
    let t444 = b17_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t443, 4u32);
    let t445: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t444[0]), vmovn_u64(t444[1])), vcombine_u32(vmovn_u64(t444[2]), vmovn_u64(t444[3]))];
    x64 = t445;
    let t446 = b13_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x72 = ((t446).wrapping_add((256u32).wrapping_sub(1u32))).wrapping_shr((8u32) as u32);
    let t447: [uint32x4_t; 2] = [vmulq_u32(x48[0], t438[0]), vmulq_u32(x48[1], t438[1])];
    let t448 = [vdupq_n_u32(x72); 2];
    let t449: [uint32x4_t; 2] = [vsubq_u32(t448[0], t447[0]), vsubq_u32(t448[1], t447[1])];
    let t450 = b18_sl_p(l, lid0, live, 4u32, t449);
    x80 = t450;
    let t451: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t452 = [vdupq_n_u32((4u32).wrapping_sub(1u32)); 2];
    let t453: [uint32x4_t; 2] = [vandq_u32(t451[0], t452[0]), vandq_u32(t451[1], t452[1])];
    x88 = t453;
    let t454 = [vdupq_n_u32(0u32); 2];
    let t455: [uint32x4_t; 2] = [vceqq_u32(x88[0], t454[0]), vceqq_u32(x88[1], t454[1])];
    let mut t456: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t457: [uint32x4_t; 2] = [vandq_u32(live[0], t455[0]), vandq_u32(live[1], t455[1])];
    let t458: [uint32x4_t; 2] = [vbicq_u32(live[0], t455[0]), vbicq_u32(live[1], t455[1])];
    {
        t456 = [vbslq_u32(t457[0], t454[0], t456[0]), vbslq_u32(t457[1], t454[1], t456[1])];
    }
    if any(&t458) {
        let t459 = [vdupq_n_u32(1u32); 2];
        let t460: [uint32x4_t; 2] = [vsubq_u32(x88[0], t459[0]), vsubq_u32(x88[1], t459[1])];
        let t461 = [vdupq_n_u32(2u32); 2];
        let t462: [uint32x4_t; 2] = [vmulq_u32(t461[0], t460[0]), vmulq_u32(t461[1], t460[1])];
        let t463: [uint32x4_t; 2] = [vaddq_u32(x56[0], t462[0]), vaddq_u32(x56[1], t462[1])];
        let t464 = b19_ll_p(l, lid0, t458, t463, x64);
        let t465 = b17_r0r1sls_p(l, lid0, t458, arg0, arg1, x32, t464, 2u32);
        let t466: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t465[0]), vmovn_u64(t465[1])), vcombine_u32(vmovn_u64(t465[2]), vmovn_u64(t465[3]))];
        t456 = [vbslq_u32(t458[0], t466[0], t456[0]), vbslq_u32(t458[1], t466[1], t456[1])];
    }
    x96 = t456;
    let t467 = [vdupq_n_u32(2u32); 2];
    let t468: [uint32x4_t; 2] = [vmulq_u32(t467[0], x88[0]), vmulq_u32(t467[1], x88[1])];
    let t469: [uint32x4_t; 2] = [vaddq_u32(x56[0], t468[0]), vaddq_u32(x56[1], t468[1])];
    let t470 = b19_ll_p(l, lid0, live, t469, x64);
    let t471 = b17_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t470, 2u32);
    let t472: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t471[0]), vmovn_u64(t471[1])), vcombine_u32(vmovn_u64(t471[2]), vmovn_u64(t471[3]))];
    x104 = t472;
    let t473: [uint32x4_t; 2] = [vmulq_u32(t467[0], x80[0]), vmulq_u32(t467[1], x80[1])];
    let t474: [uint32x4_t; 2] = [vaddq_u32(x56[0], t473[0]), vaddq_u32(x56[1], t473[1])];
    x112 = t474;
    let t475 = [vdupq_n_u32((256u32).wrapping_sub(1u32)); 2];
    let t476: [uint32x4_t; 2] = [vandq_u32(x24[0], t475[0]), vandq_u32(x24[1], t475[1])];
    x120 = t476;
    x128 = 9259542123273814144u64;
    let t477: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x120[0])), vmovl_u32(vget_high_u32(x120[0])), vmovl_u32(vget_low_u32(x120[1])), vmovl_u32(vget_high_u32(x120[1]))];
    let t478 = [vdupq_n_u64(72340172838076673u64); 4];
    let mut t479: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    t479[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t477[0]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t478[0]) as u64) as u64, t479[0]);
    t479[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t477[0]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t478[0]) as u64) as u64, t479[0]);
    t479[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t477[1]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t478[1]) as u64) as u64, t479[1]);
    t479[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t477[1]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t478[1]) as u64) as u64, t479[1]);
    t479[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t477[2]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t478[2]) as u64) as u64, t479[2]);
    t479[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t477[2]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t478[2]) as u64) as u64, t479[2]);
    t479[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t477[3]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t478[3]) as u64) as u64, t479[3]);
    t479[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t477[3]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t478[3]) as u64) as u64, t479[3]);
    x136 = t479;
    x144 = x96;
    x152 = x96;
    let mut t480 = live;
    loop {
        // Lanes part: each its own rounds.
        let t482: [uint32x4_t; 2] = [vcltq_u32(x152[0], x104[0]), vcltq_u32(x152[1], x104[1])];
        t480 = [vandq_u32(t480[0], t482[0]), vandq_u32(t480[1], t482[1])];
        if !(any(&t480)) { break; }
        let t484: [uint32x4_t; 2] = [vaddq_u32(x112[0], x152[0]), vaddq_u32(x112[1], x152[1])];
        let t485 = b19_ll_p(l, lid0, t480, t484, x64);
        let t486 = b17_r0r1sls_p(l, lid0, t480, arg0, arg1, x32, t485, 8u32);
        x160 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t480[0])))), t486[0], x160[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t480[0])))), t486[1], x160[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t480[1])))), t486[2], x160[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t480[1])))), t486[3], x160[3])];
        let t487 = [vdupq_n_u64(x128); 4];
        let t488: [uint64x2_t; 4] = [vorrq_u64(x160[0], t487[0]), vorrq_u64(x160[1], t487[1]), vorrq_u64(x160[2], t487[2]), vorrq_u64(x160[3], t487[3])];
        let t489 = [vdupq_n_u64(!(x128)); 4];
        let t490: [uint64x2_t; 4] = [vandq_u64(x136[0], t489[0]), vandq_u64(x136[1], t489[1]), vandq_u64(x136[2], t489[2]), vandq_u64(x136[3], t489[3])];
        let t491: [uint64x2_t; 4] = [vsubq_u64(t488[0], t490[0]), vsubq_u64(t488[1], t490[1]), vsubq_u64(t488[2], t490[2]), vsubq_u64(t488[3], t490[3])];
        x168 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t480[0])))), t491[0], x168[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t480[0])))), t491[1], x168[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t480[1])))), t491[2], x168[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t480[1])))), t491[3], x168[3])];
        let t492: [uint64x2_t; 4] = [vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[0]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[1]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[2]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[3])))];
        let t493: [uint64x2_t; 4] = [vandq_u64(t492[0], x136[0]), vandq_u64(t492[1], x136[1]), vandq_u64(t492[2], x136[2]), vandq_u64(t492[3], x136[3])];
        let t494: [uint64x2_t; 4] = [veorq_u64(x160[0], x136[0]), veorq_u64(x160[1], x136[1]), veorq_u64(x160[2], x136[2]), veorq_u64(x160[3], x136[3])];
        let t495: [uint64x2_t; 4] = [vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t494[0]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t494[1]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t494[2]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t494[3])))];
        let t496: [uint64x2_t; 4] = [vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[0]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[1]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[2]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[3])))];
        let t497: [uint64x2_t; 4] = [vandq_u64(t495[0], t496[0]), vandq_u64(t495[1], t496[1]), vandq_u64(t495[2], t496[2]), vandq_u64(t495[3], t496[3])];
        let t498: [uint64x2_t; 4] = [vorrq_u64(t493[0], t497[0]), vorrq_u64(t493[1], t497[1]), vorrq_u64(t493[2], t497[2]), vorrq_u64(t493[3], t497[3])];
        let t499 = [vdupq_n_u64(x128); 4];
        let t500: [uint64x2_t; 4] = [vandq_u64(t499[0], t498[0]), vandq_u64(t499[1], t498[1]), vandq_u64(t499[2], t498[2]), vandq_u64(t499[3], t498[3])];
        x176 = t500;
        let t501: [uint32x4_t; 2] = [vsubq_u32(x104[0], x152[0]), vsubq_u32(x104[1], x152[1])];
        x184 = t501;
        let t502 = [vdupq_n_u32(8u32); 2];
        let t503: [uint32x4_t; 2] = [vcgeq_u32(x184[0], t502[0]), vcgeq_u32(x184[1], t502[1])];
        let mut t504: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
        let t505: [uint32x4_t; 2] = [vandq_u32(t480[0], t503[0]), vandq_u32(t480[1], t503[1])];
        let t506: [uint32x4_t; 2] = [vbicq_u32(t480[0], t503[0]), vbicq_u32(t480[1], t503[1])];
        {
            t504 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t505[0])))), x176[0], t504[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t505[0])))), x176[1], t504[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t505[1])))), x176[2], t504[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t505[1])))), x176[3], t504[3])];
        }
        {
            let t507: [uint32x4_t; 2] = [vmulq_u32(t502[0], x184[0]), vmulq_u32(t502[1], x184[1])];
            let t508 = [vdupq_n_u64(1u64); 4];
            let t509: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t507[0])), vmovl_u32(vget_high_u32(t507[0])), vmovl_u32(vget_low_u32(t507[1])), vmovl_u32(vget_high_u32(t507[1]))];
            let t510: [uint64x2_t; 4] = [vshlq_u64(t508[0], vreinterpretq_s64_u64(vandq_u64(t509[0], vdupq_n_u64(63)))), vshlq_u64(t508[1], vreinterpretq_s64_u64(vandq_u64(t509[1], vdupq_n_u64(63)))), vshlq_u64(t508[2], vreinterpretq_s64_u64(vandq_u64(t509[2], vdupq_n_u64(63)))), vshlq_u64(t508[3], vreinterpretq_s64_u64(vandq_u64(t509[3], vdupq_n_u64(63))))];
            let t511: [uint64x2_t; 4] = [vsubq_u64(t510[0], t508[0]), vsubq_u64(t510[1], t508[1]), vsubq_u64(t510[2], t508[2]), vsubq_u64(t510[3], t508[3])];
            let t512: [uint64x2_t; 4] = [vandq_u64(x176[0], t511[0]), vandq_u64(x176[1], t511[1]), vandq_u64(x176[2], t511[2]), vandq_u64(x176[3], t511[3])];
            t504 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t506[0])))), t512[0], t504[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t506[0])))), t512[1], t504[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t506[1])))), t512[2], t504[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t506[1])))), t512[3], t504[3])];
        }
        let t513 = b20_l_p(l, lid0, t480, t504);
        let t514: [uint32x4_t; 2] = [vaddq_u32(x144[0], t513[0]), vaddq_u32(x144[1], t513[1])];
        x144 = [vbslq_u32(t480[0], t514[0], x144[0]), vbslq_u32(t480[1], t514[1], x144[1])];
        let t515: [uint32x4_t; 2] = [vandq_u32(t480[0], live[0]), vandq_u32(t480[1], live[1])];
        x152 = [vaddq_u32(x152[0], vandq_u32(t515[0], vdupq_n_u32(8))), vaddq_u32(x152[1], vandq_u32(t515[1], vdupq_n_u32(8)))];
    }
    x96 = x144;
    let t516 = b13_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    let t517 = [vdupq_n_u32((((t516).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32))); 2];
    x152 = t517;
    let t518 = [vdupq_n_u32(1u32); 2];
    let t519: [uint32x4_t; 2] = [vsubq_u32(x80[0], t518[0]), vsubq_u32(x80[1], t518[1])];
    let t520: [uint32x4_t; 2] = [vmulq_u32(t467[0], t519[0]), vmulq_u32(t467[1], t519[1])];
    let t521: [uint32x4_t; 2] = [vaddq_u32(x56[0], t520[0]), vaddq_u32(x56[1], t520[1])];
    let t522 = b19_ll_p(l, lid0, live, t521, x64);
    let t523 = b17_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t522, 2u32);
    let t524: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t523[0]), vmovn_u64(t523[1])), vcombine_u32(vmovn_u64(t523[2]), vmovn_u64(t523[3]))];
    let t525: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t524[0])), vmovl_u32(vget_high_u32(t524[0])), vmovl_u32(vget_low_u32(t524[1])), vmovl_u32(vget_high_u32(t524[1]))];
    x160 = t525;
    let t526: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x160[0]), vmovn_u64(x160[1])), vcombine_u32(vmovn_u64(x160[2]), vmovn_u64(x160[3]))];
    let t527: [uint32x4_t; 2] = [vaddq_u32(x112[0], t526[0]), vaddq_u32(x112[1], t526[1])];
    let t528: [uint32x4_t; 2] = [vmulq_u32(x96[0], x152[0]), vmulq_u32(x96[1], x152[1])];
    let t529: [uint32x4_t; 2] = [vaddq_u32(t527[0], t528[0]), vaddq_u32(t527[1], t528[1])];
    let t530 = b19_ll_p(l, lid0, live, t529, x64);
    let t531: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t530[0])), vmovl_u32(vget_high_u32(t530[0])), vmovl_u32(vget_low_u32(t530[1])), vmovl_u32(vget_high_u32(t530[1]))];
    x168 = t531;
    let t532: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x168[0]), vmovn_u64(x168[1])), vcombine_u32(vmovn_u64(x168[2]), vmovn_u64(x168[3]))];
    let t533 = b17_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t532, 8u32);
    let t534 = b10_l_p(l, lid0, live, x152);
    let t535: [uint64x2_t; 4] = [vandq_u64(t533[0], t534[0]), vandq_u64(t533[1], t534[1]), vandq_u64(t533[2], t534[2]), vandq_u64(t533[3], t534[3])];
    res = t535;
    return res;
}

#[inline(always)]
unsafe fn b13_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    res = (arg1.wrapping_add(((((x16).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
    return res;
}

#[inline(always)]
unsafe fn b13_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    res = (arg1.wrapping_add(((((x16).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
    return res;
}

#[inline(always)]
unsafe fn b14_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t536 = b13_r0r1s_p(l, lid0, live, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t536;
    return res;
}

#[inline(always)]
unsafe fn b14_r0r1s_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t537 = b13_r0r1s_f(l, lid0, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t537;
    return res;
}

#[inline(always)]
unsafe fn b15_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t538 = [vdupq_n_u32(x8); 2];
    let t539: [uint32x4_t; 2] = [vcltq_u32(x0[0], t538[0]), vcltq_u32(x0[1], t538[1])];
    let mut t540: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t541: [uint32x4_t; 2] = [vandq_u32(live[0], t539[0]), vandq_u32(live[1], t539[1])];
    let t542: [uint32x4_t; 2] = [vbicq_u32(live[0], t539[0]), vbicq_u32(live[1], t539[1])];
    {
        t540 = [vbslq_u32(t541[0], x0[0], t540[0]), vbslq_u32(t541[1], x0[1], t540[1])];
    }
    {
        let t543 = [vdupq_n_u32(x8); 2];
        t540 = [vbslq_u32(t542[0], t543[0], t540[0]), vbslq_u32(t542[1], t543[1], t540[1])];
    }
    res = t540;
    return res;
}

#[inline(always)]
unsafe fn b15_ls_f(l: &Launch, lid0: u32, x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t544 = [vdupq_n_u32(x8); 2];
    let t545: [uint32x4_t; 2] = [vcltq_u32(x0[0], t544[0]), vcltq_u32(x0[1], t544[1])];
    let mut t546: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t547: [uint32x4_t; 2] = [vmvnq_u32(t545[0]), vmvnq_u32(t545[1])];
    {
        t546 = [vbslq_u32(t545[0], x0[0], t546[0]), vbslq_u32(t545[1], x0[1], t546[1])];
    }
    {
        let t548 = [vdupq_n_u32(x8); 2];
        t546 = [vbslq_u32(t547[0], t548[0], t546[0]), vbslq_u32(t547[1], t548[1], t546[1])];
    }
    res = t546;
    return res;
}

#[inline(always)]
unsafe fn b16_r0lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t549: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x8[0])), vmovl_u32(vget_high_u32(x8[0])), vmovl_u32(vget_low_u32(x8[1])), vmovl_u32(vget_high_u32(x8[1]))];
    let t550: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x16[0])), vmovl_u32(vget_high_u32(x16[0])), vmovl_u32(vget_low_u32(x16[1])), vmovl_u32(vget_high_u32(x16[1]))];
    let t551: [uint64x2_t; 4] = [vaddq_u64(t549[0], t550[0]), vaddq_u64(t549[1], t550[1]), vaddq_u64(t549[2], t550[2]), vaddq_u64(t549[3], t550[3])];
    let t552 = b21_r0ls_p(l, lid0, live, arg0, t551, x24);
    x32 = t552;
    let t553: [uint64x2_t; 4];
    if ((x24) >= (8u32)) {
        t553 = x32;
    } else {
        let t554 = b22_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t555 = [vdupq_n_u64(t554); 4];
        let t556: [uint64x2_t; 4] = [vandq_u64(x32[0], t555[0]), vandq_u64(x32[1], t555[1]), vandq_u64(x32[2], t555[2]), vandq_u64(x32[3], t555[3])];
        t553 = t556;
    }
    res = t553;
    return res;
}

#[inline(always)]
unsafe fn b16_r0lls_f(l: &Launch, lid0: u32, arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t557: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x8[0])), vmovl_u32(vget_high_u32(x8[0])), vmovl_u32(vget_low_u32(x8[1])), vmovl_u32(vget_high_u32(x8[1]))];
    let t558: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x16[0])), vmovl_u32(vget_high_u32(x16[0])), vmovl_u32(vget_low_u32(x16[1])), vmovl_u32(vget_high_u32(x16[1]))];
    let t559: [uint64x2_t; 4] = [vaddq_u64(t557[0], t558[0]), vaddq_u64(t557[1], t558[1]), vaddq_u64(t557[2], t558[2]), vaddq_u64(t557[3], t558[3])];
    let t560 = b21_r0ls_f(l, lid0, arg0, t559, x24);
    x32 = t560;
    let t561: [uint64x2_t; 4];
    if ((x24) >= (8u32)) {
        t561 = x32;
    } else {
        let t562 = b22_s_f(l, lid0, (8u32).wrapping_mul(x24));
        let t563 = [vdupq_n_u64(t562); 4];
        let t564: [uint64x2_t; 4] = [vandq_u64(x32[0], t563[0]), vandq_u64(x32[1], t563[1]), vandq_u64(x32[2], t563[2]), vandq_u64(x32[3], t563[3])];
        t561 = t564;
    }
    res = t561;
    return res;
}

#[inline(always)]
unsafe fn b17_r0r1sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2], x32: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t565 = b23_r0sls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t565;
    return res;
}

#[inline(always)]
unsafe fn b18_sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t566 = [vdupq_n_u32(x0); 2];
    let t567: [uint32x4_t; 2] = [vcltq_u32(t566[0], x8[0]), vcltq_u32(t566[1], x8[1])];
    let mut t568: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t569: [uint32x4_t; 2] = [vandq_u32(live[0], t567[0]), vandq_u32(live[1], t567[1])];
    let t570: [uint32x4_t; 2] = [vbicq_u32(live[0], t567[0]), vbicq_u32(live[1], t567[1])];
    {
        let t571 = [vdupq_n_u32(x0); 2];
        t568 = [vbslq_u32(t569[0], t571[0], t568[0]), vbslq_u32(t569[1], t571[1], t568[1])];
    }
    {
        t568 = [vbslq_u32(t570[0], x8[0], t568[0]), vbslq_u32(t570[1], x8[1], t568[1])];
    }
    res = t568;
    return res;
}

#[inline(always)]
unsafe fn b19_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t572: [uint32x4_t; 2] = [vcltq_u32(x0[0], x8[0]), vcltq_u32(x0[1], x8[1])];
    let mut t573: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t574: [uint32x4_t; 2] = [vandq_u32(live[0], t572[0]), vandq_u32(live[1], t572[1])];
    let t575: [uint32x4_t; 2] = [vbicq_u32(live[0], t572[0]), vbicq_u32(live[1], t572[1])];
    {
        t573 = [vbslq_u32(t574[0], x0[0], t573[0]), vbslq_u32(t574[1], x0[1], t573[1])];
    }
    {
        t573 = [vbslq_u32(t575[0], x8[0], t573[0]), vbslq_u32(t575[1], x8[1], t573[1])];
    }
    res = t573;
    return res;
}

#[inline(always)]
unsafe fn b20_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint64x2_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t576: [uint64x2_t; 4] = [vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[0]))))), vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[1]))))), vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[2]))))), vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[3])))))];
    let t577: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t576[0]), vmovn_u64(t576[1])), vcombine_u32(vmovn_u64(t576[2]), vmovn_u64(t576[3]))];
    res = t577;
    return res;
}

#[inline(always)]
unsafe fn b21_r0ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint64x2_t; 4], x16: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t578 = [vdupq_n_u64(!(7u64)); 4];
    let t579: [uint64x2_t; 4] = [vandq_u64(x8[0], t578[0]), vandq_u64(x8[1], t578[1]), vandq_u64(x8[2], t578[2]), vandq_u64(x8[3], t578[3])];
    let t580: [uint64x2_t; 4] = [vandq_u64(t579[0], vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0]))))), vandq_u64(t579[1], vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0]))))), vandq_u64(t579[2], vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1]))))), vandq_u64(t579[3], vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))))];
    let t581: [uint64x2_t; 4] = [vaddq_u64(vdupq_n_u64((arg0) as u64), t580[0]), vaddq_u64(vdupq_n_u64((arg0) as u64), t580[1]), vaddq_u64(vdupq_n_u64((arg0) as u64), t580[2]), vaddq_u64(vdupq_n_u64((arg0) as u64), t580[3])];
    x24 = t581;
    let t582 = [vdupq_n_u64(7u64); 4];
    let t583: [uint64x2_t; 4] = [vandq_u64(x8[0], t582[0]), vandq_u64(x8[1], t582[1]), vandq_u64(x8[2], t582[2]), vandq_u64(x8[3], t582[3])];
    let t584: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t583[0]), vmovn_u64(t583[1])), vcombine_u32(vmovn_u64(t583[2]), vmovn_u64(t583[3]))];
    let t585 = [vdupq_n_u32(8u32); 2];
    let t586: [uint32x4_t; 2] = [vmulq_u32(t584[0], t585[0]), vmulq_u32(t584[1], t585[1])];
    x32 = t586;
    let mut t587: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    t587[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[0]) as usize as *const u64).read_unaligned(), t587[0]);
    t587[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[0]) as usize as *const u64).read_unaligned(), t587[0]);
    t587[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[1]) as usize as *const u64).read_unaligned(), t587[1]);
    t587[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[1]) as usize as *const u64).read_unaligned(), t587[1]);
    t587[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[2]) as usize as *const u64).read_unaligned(), t587[2]);
    t587[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[2]) as usize as *const u64).read_unaligned(), t587[2]);
    t587[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[3]) as usize as *const u64).read_unaligned(), t587[3]);
    t587[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[3]) as usize as *const u64).read_unaligned(), t587[3]);
    let t588: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x32[0])), vmovl_u32(vget_high_u32(x32[0])), vmovl_u32(vget_low_u32(x32[1])), vmovl_u32(vget_high_u32(x32[1]))];
    let t589: [uint64x2_t; 4] = [vshlq_u64(t587[0], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t588[0], vdupq_n_u64(63))))), vshlq_u64(t587[1], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t588[1], vdupq_n_u64(63))))), vshlq_u64(t587[2], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t588[2], vdupq_n_u64(63))))), vshlq_u64(t587[3], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t588[3], vdupq_n_u64(63)))))];
    x40 = t589;
    let t590 = [vdupq_n_u32(0u32); 2];
    let t591: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t590[0])), vmvnq_u32(vceqq_u32(x32[1], t590[1]))];
    let t592 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t593: [uint32x4_t; 2] = [vaddq_u32(x32[0], t592[0]), vaddq_u32(x32[1], t592[1])];
    let t594 = [vdupq_n_u32(64u32); 2];
    let t595: [uint32x4_t; 2] = [vcgtq_u32(t593[0], t594[0]), vcgtq_u32(t593[1], t594[1])];
    let t596: [uint32x4_t; 2] = [vandq_u32(t591[0], t595[0]), vandq_u32(t591[1], t595[1])];
    let mut t597: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t598: [uint32x4_t; 2] = [vandq_u32(live[0], t596[0]), vandq_u32(live[1], t596[1])];
    let t599: [uint32x4_t; 2] = [vbicq_u32(live[0], t596[0]), vbicq_u32(live[1], t596[1])];
    if any(&t598) {
        let t600 = [vdupq_n_u64((1u64).wrapping_mul(8u64)); 4];
        let t601: [uint64x2_t; 4] = [vaddq_u64(x24[0], t600[0]), vaddq_u64(x24[1], t600[1]), vaddq_u64(x24[2], t600[2]), vaddq_u64(x24[3], t600[3])];
        let mut t602: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
        t602[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t601[0]) as usize as *const u64).read_unaligned(), t602[0]);
        t602[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t601[0]) as usize as *const u64).read_unaligned(), t602[0]);
        t602[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t601[1]) as usize as *const u64).read_unaligned(), t602[1]);
        t602[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t601[1]) as usize as *const u64).read_unaligned(), t602[1]);
        t602[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t601[2]) as usize as *const u64).read_unaligned(), t602[2]);
        t602[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t601[2]) as usize as *const u64).read_unaligned(), t602[2]);
        t602[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t601[3]) as usize as *const u64).read_unaligned(), t602[3]);
        t602[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t601[3]) as usize as *const u64).read_unaligned(), t602[3]);
        let t603: [uint32x4_t; 2] = [vsubq_u32(t594[0], x32[0]), vsubq_u32(t594[1], x32[1])];
        let t604: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t603[0])), vmovl_u32(vget_high_u32(t603[0])), vmovl_u32(vget_low_u32(t603[1])), vmovl_u32(vget_high_u32(t603[1]))];
        let t605: [uint64x2_t; 4] = [vshlq_u64(t602[0], vreinterpretq_s64_u64(vandq_u64(t604[0], vdupq_n_u64(63)))), vshlq_u64(t602[1], vreinterpretq_s64_u64(vandq_u64(t604[1], vdupq_n_u64(63)))), vshlq_u64(t602[2], vreinterpretq_s64_u64(vandq_u64(t604[2], vdupq_n_u64(63)))), vshlq_u64(t602[3], vreinterpretq_s64_u64(vandq_u64(t604[3], vdupq_n_u64(63))))];
        let t606: [uint64x2_t; 4] = [vorrq_u64(x40[0], t605[0]), vorrq_u64(x40[1], t605[1]), vorrq_u64(x40[2], t605[2]), vorrq_u64(x40[3], t605[3])];
        t597 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t598[0])))), t606[0], t597[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t598[0])))), t606[1], t597[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t598[1])))), t606[2], t597[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t598[1])))), t606[3], t597[3])];
    }
    {
        t597 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t599[0])))), x40[0], t597[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t599[0])))), x40[1], t597[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t599[1])))), x40[2], t597[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t599[1])))), x40[3], t597[3])];
    }
    res = t597;
    return res;
}

#[inline(always)]
unsafe fn b21_r0ls_f(l: &Launch, lid0: u32, arg0: *const u8, x8: [uint64x2_t; 4], x16: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t607 = [vdupq_n_u64(!(7u64)); 4];
    let t608: [uint64x2_t; 4] = [vandq_u64(x8[0], t607[0]), vandq_u64(x8[1], t607[1]), vandq_u64(x8[2], t607[2]), vandq_u64(x8[3], t607[3])];
    let t609: [uint64x2_t; 4] = [vaddq_u64(vdupq_n_u64((arg0) as u64), t608[0]), vaddq_u64(vdupq_n_u64((arg0) as u64), t608[1]), vaddq_u64(vdupq_n_u64((arg0) as u64), t608[2]), vaddq_u64(vdupq_n_u64((arg0) as u64), t608[3])];
    x24 = t609;
    let t610 = [vdupq_n_u64(7u64); 4];
    let t611: [uint64x2_t; 4] = [vandq_u64(x8[0], t610[0]), vandq_u64(x8[1], t610[1]), vandq_u64(x8[2], t610[2]), vandq_u64(x8[3], t610[3])];
    let t612: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t611[0]), vmovn_u64(t611[1])), vcombine_u32(vmovn_u64(t611[2]), vmovn_u64(t611[3]))];
    let t613 = [vdupq_n_u32(8u32); 2];
    let t614: [uint32x4_t; 2] = [vmulq_u32(t612[0], t613[0]), vmulq_u32(t612[1], t613[1])];
    x32 = t614;
    let mut t615: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    t615[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[0]) as usize as *const u64).read_unaligned(), t615[0]);
    t615[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[0]) as usize as *const u64).read_unaligned(), t615[0]);
    t615[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[1]) as usize as *const u64).read_unaligned(), t615[1]);
    t615[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[1]) as usize as *const u64).read_unaligned(), t615[1]);
    t615[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[2]) as usize as *const u64).read_unaligned(), t615[2]);
    t615[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[2]) as usize as *const u64).read_unaligned(), t615[2]);
    t615[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[3]) as usize as *const u64).read_unaligned(), t615[3]);
    t615[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[3]) as usize as *const u64).read_unaligned(), t615[3]);
    let t616: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x32[0])), vmovl_u32(vget_high_u32(x32[0])), vmovl_u32(vget_low_u32(x32[1])), vmovl_u32(vget_high_u32(x32[1]))];
    let t617: [uint64x2_t; 4] = [vshlq_u64(t615[0], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t616[0], vdupq_n_u64(63))))), vshlq_u64(t615[1], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t616[1], vdupq_n_u64(63))))), vshlq_u64(t615[2], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t616[2], vdupq_n_u64(63))))), vshlq_u64(t615[3], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t616[3], vdupq_n_u64(63)))))];
    x40 = t617;
    let t618 = [vdupq_n_u32(0u32); 2];
    let t619: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t618[0])), vmvnq_u32(vceqq_u32(x32[1], t618[1]))];
    let t620 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t621: [uint32x4_t; 2] = [vaddq_u32(x32[0], t620[0]), vaddq_u32(x32[1], t620[1])];
    let t622 = [vdupq_n_u32(64u32); 2];
    let t623: [uint32x4_t; 2] = [vcgtq_u32(t621[0], t622[0]), vcgtq_u32(t621[1], t622[1])];
    let t624: [uint32x4_t; 2] = [vandq_u32(t619[0], t623[0]), vandq_u32(t619[1], t623[1])];
    let mut t625: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t626: [uint32x4_t; 2] = [vmvnq_u32(t624[0]), vmvnq_u32(t624[1])];
    if any(&t624) {
        let t627 = [vdupq_n_u64((1u64).wrapping_mul(8u64)); 4];
        let t628: [uint64x2_t; 4] = [vaddq_u64(x24[0], t627[0]), vaddq_u64(x24[1], t627[1]), vaddq_u64(x24[2], t627[2]), vaddq_u64(x24[3], t627[3])];
        let mut t629: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
        t629[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t628[0]) as usize as *const u64).read_unaligned(), t629[0]);
        t629[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t628[0]) as usize as *const u64).read_unaligned(), t629[0]);
        t629[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t628[1]) as usize as *const u64).read_unaligned(), t629[1]);
        t629[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t628[1]) as usize as *const u64).read_unaligned(), t629[1]);
        t629[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t628[2]) as usize as *const u64).read_unaligned(), t629[2]);
        t629[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t628[2]) as usize as *const u64).read_unaligned(), t629[2]);
        t629[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t628[3]) as usize as *const u64).read_unaligned(), t629[3]);
        t629[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t628[3]) as usize as *const u64).read_unaligned(), t629[3]);
        let t630: [uint32x4_t; 2] = [vsubq_u32(t622[0], x32[0]), vsubq_u32(t622[1], x32[1])];
        let t631: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t630[0])), vmovl_u32(vget_high_u32(t630[0])), vmovl_u32(vget_low_u32(t630[1])), vmovl_u32(vget_high_u32(t630[1]))];
        let t632: [uint64x2_t; 4] = [vshlq_u64(t629[0], vreinterpretq_s64_u64(vandq_u64(t631[0], vdupq_n_u64(63)))), vshlq_u64(t629[1], vreinterpretq_s64_u64(vandq_u64(t631[1], vdupq_n_u64(63)))), vshlq_u64(t629[2], vreinterpretq_s64_u64(vandq_u64(t631[2], vdupq_n_u64(63)))), vshlq_u64(t629[3], vreinterpretq_s64_u64(vandq_u64(t631[3], vdupq_n_u64(63))))];
        let t633: [uint64x2_t; 4] = [vorrq_u64(x40[0], t632[0]), vorrq_u64(x40[1], t632[1]), vorrq_u64(x40[2], t632[2]), vorrq_u64(x40[3], t632[3])];
        t625 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t624[0])))), t633[0], t625[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t624[0])))), t633[1], t625[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t624[1])))), t633[2], t625[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t624[1])))), t633[3], t625[3])];
    }
    {
        t625 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t626[0])))), x40[0], t625[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t626[0])))), x40[1], t625[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t626[1])))), x40[2], t625[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t626[1])))), x40[3], t625[3])];
    }
    res = t625;
    return res;
}

#[inline(always)]
unsafe fn b22_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t634: u64;
    if ((x0) >= (64u32)) {
        t634 = !(0u64);
    } else {
        t634 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t634;
    return res;
}

#[inline(always)]
unsafe fn b22_s_f(l: &Launch, lid0: u32, x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u64 = 0;
    let t635: u64;
    if ((x0) >= (64u32)) {
        t635 = !(0u64);
    } else {
        t635 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t635;
    return res;
}

#[inline(always)]
unsafe fn b23_r0sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16: [uint32x4_t; 2], x24: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t636 = [vdupq_n_u64(((x8) as u64)); 4];
    let t637: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x16[0])), vmovl_u32(vget_high_u32(x16[0])), vmovl_u32(vget_low_u32(x16[1])), vmovl_u32(vget_high_u32(x16[1]))];
    let t638: [uint64x2_t; 4] = [vaddq_u64(t636[0], t637[0]), vaddq_u64(t636[1], t637[1]), vaddq_u64(t636[2], t637[2]), vaddq_u64(t636[3], t637[3])];
    let t639 = b21_r0ls_p(l, lid0, live, arg0, t638, x24);
    x32 = t639;
    let t640: [uint64x2_t; 4];
    if ((x24) >= (8u32)) {
        t640 = x32;
    } else {
        let t641 = b22_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t642 = [vdupq_n_u64(t641); 4];
        let t643: [uint64x2_t; 4] = [vandq_u64(x32[0], t642[0]), vandq_u64(x32[1], t642[1]), vandq_u64(x32[2], t642[2]), vandq_u64(x32[3], t642[3])];
        t640 = t643;
    }
    res = t640;
    return res;
}

/// The group `l.group`'s threads, 8 at a time: `argv[j]` the address of argument `j`'s bytes (a buffer's address,
/// a value's bytes).
///
/// # Safety
///
/// `argv` holds the kernel's arguments, its buffers as large as it reads and writes; `l` is a 1-D group's launch.
pub unsafe fn parent_back_w8(argv: *const *const u8, l: &Launch) {
    let n = l.block[0] * l.block[1] * l.block[2];
    let arg0: *const u8 = (*argv.add(0) as *const *const u8).read_unaligned();
    let arg1: *const u8 = (*argv.add(1) as *const *const u8).read_unaligned();
    let arg2: *const u8 = (*argv.add(2) as *const *const u8).read_unaligned();
    let arg3: *const u8 = (*argv.add(3) as *const *const u8).read_unaligned();
    let x32: *const u8 = *argv.add(4);
    let arg5: *const u8 = (*argv.add(5) as *const *const u8).read_unaligned();
    let x56: u32 = (*argv.add(6) as *const u32).read_unaligned();
    let mut t644: bool = !(arg5).is_null();
    if t644 {
        let mut t645: bool = ((l.group[0]) >= ((arg5.wrapping_add((((x56) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t645 {
            t645 = ((l.group[1]) >= ((arg5.wrapping_add(((((x56).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t646: bool = t645;
        if !t646 {
            t646 = ((l.group[2]) >= ((arg5.wrapping_add(((((x56).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t644 = t646;
    }
    if t644 {
        return;
    }
    let mut lid0 = 0u32;
    while lid0 + 8 <= n {
        let mut live = [vdupq_n_u32(u32::MAX); 2];
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1r2r3sgi_f(l, lid0, arg0, arg1, arg2, arg3, x32, lid0);
        }
        lid0 += 8;
    }
    if lid0 < n {
        // Lanes part: the group's last threads, those under `n`.
        let t647: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t647;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1r2r3sgi_p(l, lid0, live, arg0, arg1, arg2, arg3, x32, lid0);
        }
    }
}
