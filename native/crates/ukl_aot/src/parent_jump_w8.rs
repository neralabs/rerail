//! `parent_jump` at 8 lanes (2 NEON registers), written by rerail-ukl-vec's Rust (`rs.rs`) from its vector tape: do
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
unsafe fn b0_r0r1r2r3r4r5sg_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, arg4: *const u8, arg5: *const u8, x48: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x48 as *const u32).read_unaligned();
    let a1: u32 = (x48.wrapping_add(4u64 as usize) as *const u32).read_unaligned();
    let mut x72: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x80: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let t1: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t2 = [vdupq_n_u32(a0); 2];
    let t3: [uint32x4_t; 2] = [vcgeq_u32(t1[0], t2[0]), vcgeq_u32(t1[1], t2[1])];
    let t4: [uint32x4_t; 2] = [vandq_u32(live[0], t3[0]), vandq_u32(live[1], t3[1])];
    if any(&t4) {
        live = [vbicq_u32(live[0], t4[0]), vbicq_u32(live[1], t4[1])];
    }
    if any(&live) {
        let mut t5: [uint32x4_t; 2];
        if all(&live) {
            let t6 = (arg3 as *const u32).add((gid0) as usize);
            t5 = [vld1q_u32(t6.add(0)), vld1q_u32(t6.add(4))];
        } else {
            let t7 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1[0]), vandq_u32(live[1], t1[1]))));
            let t8: [uint32x4_t; 2] = [vbslq_u32(live[0], t1[0], t7), vbslq_u32(live[1], t1[1], t7)];
            let t9 = arg3 as *const u32;
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
        x72 = t5;
        let mut t11: [uint32x4_t; 2];
        if all(&live) {
            let t12 = (arg2 as *const u32).add((gid0) as usize);
            t11 = [vld1q_u32(t12.add(0)), vld1q_u32(t12.add(4))];
        } else {
            let t13 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t1[0]), vandq_u32(live[1], t1[1]))));
            let t14: [uint32x4_t; 2] = [vbslq_u32(live[0], t1[0], t13), vbslq_u32(live[1], t1[1], t13)];
            let t15 = arg2 as *const u32;
            let mut t16: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t16[0] = vld1q_lane_u32::<0>(t15.add(vgetq_lane_u32::<0>(t14[0]) as usize), t16[0]);
            t16[0] = vld1q_lane_u32::<1>(t15.add(vgetq_lane_u32::<1>(t14[0]) as usize), t16[0]);
            t16[0] = vld1q_lane_u32::<2>(t15.add(vgetq_lane_u32::<2>(t14[0]) as usize), t16[0]);
            t16[0] = vld1q_lane_u32::<3>(t15.add(vgetq_lane_u32::<3>(t14[0]) as usize), t16[0]);
            t16[1] = vld1q_lane_u32::<0>(t15.add(vgetq_lane_u32::<0>(t14[1]) as usize), t16[1]);
            t16[1] = vld1q_lane_u32::<1>(t15.add(vgetq_lane_u32::<1>(t14[1]) as usize), t16[1]);
            t16[1] = vld1q_lane_u32::<2>(t15.add(vgetq_lane_u32::<2>(t14[1]) as usize), t16[1]);
            t16[1] = vld1q_lane_u32::<3>(t15.add(vgetq_lane_u32::<3>(t14[1]) as usize), t16[1]);
            t11 = t16;
        }
        x80 = t11;
        let t17 = [vdupq_n_u32(4294967295u32); 2];
        let t18: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x72[0], t17[0])), vmvnq_u32(vceqq_u32(x72[1], t17[1]))];
        let t19: [uint32x4_t; 2] = [vandq_u32(live[0], t18[0]), vandq_u32(live[1], t18[1])];
        if any(&t19) {
            let t20 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(t19[0], x72[0]), vandq_u32(t19[1], x72[1]))));
            let t21: [uint32x4_t; 2] = [vbslq_u32(t19[0], x72[0], t20), vbslq_u32(t19[1], x72[1], t20)];
            let t22 = arg2 as *const u32;
            let mut t23: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t23[0] = vld1q_lane_u32::<0>(t22.add(vgetq_lane_u32::<0>(t21[0]) as usize), t23[0]);
            t23[0] = vld1q_lane_u32::<1>(t22.add(vgetq_lane_u32::<1>(t21[0]) as usize), t23[0]);
            t23[0] = vld1q_lane_u32::<2>(t22.add(vgetq_lane_u32::<2>(t21[0]) as usize), t23[0]);
            t23[0] = vld1q_lane_u32::<3>(t22.add(vgetq_lane_u32::<3>(t21[0]) as usize), t23[0]);
            t23[1] = vld1q_lane_u32::<0>(t22.add(vgetq_lane_u32::<0>(t21[1]) as usize), t23[1]);
            t23[1] = vld1q_lane_u32::<1>(t22.add(vgetq_lane_u32::<1>(t21[1]) as usize), t23[1]);
            t23[1] = vld1q_lane_u32::<2>(t22.add(vgetq_lane_u32::<2>(t21[1]) as usize), t23[1]);
            t23[1] = vld1q_lane_u32::<3>(t22.add(vgetq_lane_u32::<3>(t21[1]) as usize), t23[1]);
            x80 = [vbslq_u32(t19[0], t23[0], x80[0]), vbslq_u32(t19[1], t23[1], x80[1])];
            let t24 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(t19[0], x72[0]), vandq_u32(t19[1], x72[1]))));
            let t25: [uint32x4_t; 2] = [vbslq_u32(t19[0], x72[0], t24), vbslq_u32(t19[1], x72[1], t24)];
            let t26 = arg3 as *const u32;
            let mut t27: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t27[0] = vld1q_lane_u32::<0>(t26.add(vgetq_lane_u32::<0>(t25[0]) as usize), t27[0]);
            t27[0] = vld1q_lane_u32::<1>(t26.add(vgetq_lane_u32::<1>(t25[0]) as usize), t27[0]);
            t27[0] = vld1q_lane_u32::<2>(t26.add(vgetq_lane_u32::<2>(t25[0]) as usize), t27[0]);
            t27[0] = vld1q_lane_u32::<3>(t26.add(vgetq_lane_u32::<3>(t25[0]) as usize), t27[0]);
            t27[1] = vld1q_lane_u32::<0>(t26.add(vgetq_lane_u32::<0>(t25[1]) as usize), t27[1]);
            t27[1] = vld1q_lane_u32::<1>(t26.add(vgetq_lane_u32::<1>(t25[1]) as usize), t27[1]);
            t27[1] = vld1q_lane_u32::<2>(t26.add(vgetq_lane_u32::<2>(t25[1]) as usize), t27[1]);
            t27[1] = vld1q_lane_u32::<3>(t26.add(vgetq_lane_u32::<3>(t25[1]) as usize), t27[1]);
            x72 = [vbslq_u32(t19[0], t27[0], x72[0]), vbslq_u32(t19[1], t27[1], x72[1])];
        }
        let t28: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x72[0], t17[0])), vmvnq_u32(vceqq_u32(x72[1], t17[1]))];
        let t29 = [vdupq_n_u32(if ((a1) != (0u32)) { u32::MAX } else { 0 }); 2];
        let t30: [uint32x4_t; 2] = [vandq_u32(t28[0], t29[0]), vandq_u32(t28[1], t29[1])];
        let t31: [uint32x4_t; 2] = [vandq_u32(live[0], t30[0]), vandq_u32(live[1], t30[1])];
        if any(&t31) {
            let t32 = b1_r0r1g_p(l, lid0, t31, arg0, arg1);
            x80 = [vbslq_u32(t31[0], t32[0], x80[0]), vbslq_u32(t31[1], t32[1], x80[1])];
            x72 = [vbslq_u32(t31[0], t17[0], x72[0]), vbslq_u32(t31[1], t17[1], x72[1])];
        }
        let t33 = (arg4 as *mut u32).add((gid0) as usize);
        if all(&live) {
            vst1q_u32(t33.add(0), x80[0]);
            vst1q_u32(t33.add(4), x80[1]);
        } else {
            let mut t34w = 0u32;
            let t34 = core::hint::black_box(&raw mut t34w);
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t33.add(0), t34).write(vgetq_lane_u32::<0>(x80[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t33.add(1), t34).write(vgetq_lane_u32::<1>(x80[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t33.add(2), t34).write(vgetq_lane_u32::<2>(x80[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t33.add(3), t34).write(vgetq_lane_u32::<3>(x80[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t33.add(4), t34).write(vgetq_lane_u32::<0>(x80[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t33.add(5), t34).write(vgetq_lane_u32::<1>(x80[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t33.add(6), t34).write(vgetq_lane_u32::<2>(x80[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t33.add(7), t34).write(vgetq_lane_u32::<3>(x80[1]));
        }
        let t35 = (arg5 as *mut u32).add((gid0) as usize);
        if all(&live) {
            vst1q_u32(t35.add(0), x72[0]);
            vst1q_u32(t35.add(4), x72[1]);
        } else {
            let mut t36w = 0u32;
            let t36 = core::hint::black_box(&raw mut t36w);
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t35.add(0), t36).write(vgetq_lane_u32::<0>(x72[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t35.add(1), t36).write(vgetq_lane_u32::<1>(x72[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t35.add(2), t36).write(vgetq_lane_u32::<2>(x72[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t35.add(3), t36).write(vgetq_lane_u32::<3>(x72[0]));
            core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t35.add(4), t36).write(vgetq_lane_u32::<0>(x72[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t35.add(5), t36).write(vgetq_lane_u32::<1>(x72[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t35.add(6), t36).write(vgetq_lane_u32::<2>(x72[1]));
            core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t35.add(7), t36).write(vgetq_lane_u32::<3>(x72[1]));
        }
    }
}

#[inline(always)]
unsafe fn b0_r0r1r2r3r4r5sg_f(l: &Launch, lid0: u32, arg0: *const u8, arg1: *const u8, arg2: *const u8, arg3: *const u8, arg4: *const u8, arg5: *const u8, x48: *const u8) {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let a0: u32 = (x48 as *const u32).read_unaligned();
    let a1: u32 = (x48.wrapping_add(4u64 as usize) as *const u32).read_unaligned();
    let mut x72: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x80: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if ((gid0) as u64 + 7) < ((a0) as u64) {
        let t37 = (arg3 as *const u32).add((gid0) as usize);
        let t38: [uint32x4_t; 2] = [vld1q_u32(t37.add(0)), vld1q_u32(t37.add(4))];
        x72 = t38;
        let t39 = (arg2 as *const u32).add((gid0) as usize);
        let t40: [uint32x4_t; 2] = [vld1q_u32(t39.add(0)), vld1q_u32(t39.add(4))];
        x80 = t40;
        let t41 = [vdupq_n_u32(4294967295u32); 2];
        let t42: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x72[0], t41[0])), vmvnq_u32(vceqq_u32(x72[1], t41[1]))];
        {
            let t43: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
            let t44: [uint32x4_t; 2] = [vbslq_u32(t42[0], x72[0], t43[0]), vbslq_u32(t42[1], x72[1], t43[1])];
            let t45 = arg2 as *const u32;
            let mut t46: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t46[0] = vld1q_lane_u32::<0>(t45.add(vgetq_lane_u32::<0>(t44[0]) as usize), t46[0]);
            t46[0] = vld1q_lane_u32::<1>(t45.add(vgetq_lane_u32::<1>(t44[0]) as usize), t46[0]);
            t46[0] = vld1q_lane_u32::<2>(t45.add(vgetq_lane_u32::<2>(t44[0]) as usize), t46[0]);
            t46[0] = vld1q_lane_u32::<3>(t45.add(vgetq_lane_u32::<3>(t44[0]) as usize), t46[0]);
            t46[1] = vld1q_lane_u32::<0>(t45.add(vgetq_lane_u32::<0>(t44[1]) as usize), t46[1]);
            t46[1] = vld1q_lane_u32::<1>(t45.add(vgetq_lane_u32::<1>(t44[1]) as usize), t46[1]);
            t46[1] = vld1q_lane_u32::<2>(t45.add(vgetq_lane_u32::<2>(t44[1]) as usize), t46[1]);
            t46[1] = vld1q_lane_u32::<3>(t45.add(vgetq_lane_u32::<3>(t44[1]) as usize), t46[1]);
            x80 = [vbslq_u32(t42[0], t46[0], x80[0]), vbslq_u32(t42[1], t46[1], x80[1])];
            let t47: [uint32x4_t; 2] = [vbslq_u32(t42[0], x72[0], t43[0]), vbslq_u32(t42[1], x72[1], t43[1])];
            let t48 = arg3 as *const u32;
            let mut t49: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
            t49[0] = vld1q_lane_u32::<0>(t48.add(vgetq_lane_u32::<0>(t47[0]) as usize), t49[0]);
            t49[0] = vld1q_lane_u32::<1>(t48.add(vgetq_lane_u32::<1>(t47[0]) as usize), t49[0]);
            t49[0] = vld1q_lane_u32::<2>(t48.add(vgetq_lane_u32::<2>(t47[0]) as usize), t49[0]);
            t49[0] = vld1q_lane_u32::<3>(t48.add(vgetq_lane_u32::<3>(t47[0]) as usize), t49[0]);
            t49[1] = vld1q_lane_u32::<0>(t48.add(vgetq_lane_u32::<0>(t47[1]) as usize), t49[1]);
            t49[1] = vld1q_lane_u32::<1>(t48.add(vgetq_lane_u32::<1>(t47[1]) as usize), t49[1]);
            t49[1] = vld1q_lane_u32::<2>(t48.add(vgetq_lane_u32::<2>(t47[1]) as usize), t49[1]);
            t49[1] = vld1q_lane_u32::<3>(t48.add(vgetq_lane_u32::<3>(t47[1]) as usize), t49[1]);
            x72 = [vbslq_u32(t42[0], t49[0], x72[0]), vbslq_u32(t42[1], t49[1], x72[1])];
        }
        let t50: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x72[0], t41[0])), vmvnq_u32(vceqq_u32(x72[1], t41[1]))];
        let t51 = [vdupq_n_u32(if ((a1) != (0u32)) { u32::MAX } else { 0 }); 2];
        let t52: [uint32x4_t; 2] = [vandq_u32(t50[0], t51[0]), vandq_u32(t50[1], t51[1])];
        if any(&t52) {
            let t53 = b1_r0r1g_p(l, lid0, t52, arg0, arg1);
            x80 = [vbslq_u32(t52[0], t53[0], x80[0]), vbslq_u32(t52[1], t53[1], x80[1])];
            x72 = [vbslq_u32(t52[0], t41[0], x72[0]), vbslq_u32(t52[1], t41[1], x72[1])];
        }
        let t54 = (arg4 as *mut u32).add((gid0) as usize);
        vst1q_u32(t54.add(0), x80[0]);
        vst1q_u32(t54.add(4), x80[1]);
        let t55 = (arg5 as *mut u32).add((gid0) as usize);
        vst1q_u32(t55.add(0), x72[0]);
        vst1q_u32(t55.add(4), x72[1]);
    } else if (((gid0) as u64 + 7) <= u32::MAX as u64 && ((gid0) as u64) >= ((a0) as u64)) {
        return;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t56: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
        let t57 = [vdupq_n_u32(a0); 2];
        let t58: [uint32x4_t; 2] = [vcgeq_u32(t56[0], t57[0]), vcgeq_u32(t56[1], t57[1])];
        let t59: [uint32x4_t; 2] = [vandq_u32(live[0], t58[0]), vandq_u32(live[1], t58[1])];
        if any(&t59) {
            live = [vbicq_u32(live[0], t59[0]), vbicq_u32(live[1], t59[1])];
        }
        if any(&live) {
            let mut t60: [uint32x4_t; 2];
            if all(&live) {
                let t61 = (arg3 as *const u32).add((gid0) as usize);
                t60 = [vld1q_u32(t61.add(0)), vld1q_u32(t61.add(4))];
            } else {
                let t62 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t56[0]), vandq_u32(live[1], t56[1]))));
                let t63: [uint32x4_t; 2] = [vbslq_u32(live[0], t56[0], t62), vbslq_u32(live[1], t56[1], t62)];
                let t64 = arg3 as *const u32;
                let mut t65: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                t65[0] = vld1q_lane_u32::<0>(t64.add(vgetq_lane_u32::<0>(t63[0]) as usize), t65[0]);
                t65[0] = vld1q_lane_u32::<1>(t64.add(vgetq_lane_u32::<1>(t63[0]) as usize), t65[0]);
                t65[0] = vld1q_lane_u32::<2>(t64.add(vgetq_lane_u32::<2>(t63[0]) as usize), t65[0]);
                t65[0] = vld1q_lane_u32::<3>(t64.add(vgetq_lane_u32::<3>(t63[0]) as usize), t65[0]);
                t65[1] = vld1q_lane_u32::<0>(t64.add(vgetq_lane_u32::<0>(t63[1]) as usize), t65[1]);
                t65[1] = vld1q_lane_u32::<1>(t64.add(vgetq_lane_u32::<1>(t63[1]) as usize), t65[1]);
                t65[1] = vld1q_lane_u32::<2>(t64.add(vgetq_lane_u32::<2>(t63[1]) as usize), t65[1]);
                t65[1] = vld1q_lane_u32::<3>(t64.add(vgetq_lane_u32::<3>(t63[1]) as usize), t65[1]);
                t60 = t65;
            }
            x72 = t60;
            let mut t66: [uint32x4_t; 2];
            if all(&live) {
                let t67 = (arg2 as *const u32).add((gid0) as usize);
                t66 = [vld1q_u32(t67.add(0)), vld1q_u32(t67.add(4))];
            } else {
                let t68 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t56[0]), vandq_u32(live[1], t56[1]))));
                let t69: [uint32x4_t; 2] = [vbslq_u32(live[0], t56[0], t68), vbslq_u32(live[1], t56[1], t68)];
                let t70 = arg2 as *const u32;
                let mut t71: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                t71[0] = vld1q_lane_u32::<0>(t70.add(vgetq_lane_u32::<0>(t69[0]) as usize), t71[0]);
                t71[0] = vld1q_lane_u32::<1>(t70.add(vgetq_lane_u32::<1>(t69[0]) as usize), t71[0]);
                t71[0] = vld1q_lane_u32::<2>(t70.add(vgetq_lane_u32::<2>(t69[0]) as usize), t71[0]);
                t71[0] = vld1q_lane_u32::<3>(t70.add(vgetq_lane_u32::<3>(t69[0]) as usize), t71[0]);
                t71[1] = vld1q_lane_u32::<0>(t70.add(vgetq_lane_u32::<0>(t69[1]) as usize), t71[1]);
                t71[1] = vld1q_lane_u32::<1>(t70.add(vgetq_lane_u32::<1>(t69[1]) as usize), t71[1]);
                t71[1] = vld1q_lane_u32::<2>(t70.add(vgetq_lane_u32::<2>(t69[1]) as usize), t71[1]);
                t71[1] = vld1q_lane_u32::<3>(t70.add(vgetq_lane_u32::<3>(t69[1]) as usize), t71[1]);
                t66 = t71;
            }
            x80 = t66;
            let t72 = [vdupq_n_u32(4294967295u32); 2];
            let t73: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x72[0], t72[0])), vmvnq_u32(vceqq_u32(x72[1], t72[1]))];
            let t74: [uint32x4_t; 2] = [vandq_u32(live[0], t73[0]), vandq_u32(live[1], t73[1])];
            if any(&t74) {
                let t75 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(t74[0], x72[0]), vandq_u32(t74[1], x72[1]))));
                let t76: [uint32x4_t; 2] = [vbslq_u32(t74[0], x72[0], t75), vbslq_u32(t74[1], x72[1], t75)];
                let t77 = arg2 as *const u32;
                let mut t78: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                t78[0] = vld1q_lane_u32::<0>(t77.add(vgetq_lane_u32::<0>(t76[0]) as usize), t78[0]);
                t78[0] = vld1q_lane_u32::<1>(t77.add(vgetq_lane_u32::<1>(t76[0]) as usize), t78[0]);
                t78[0] = vld1q_lane_u32::<2>(t77.add(vgetq_lane_u32::<2>(t76[0]) as usize), t78[0]);
                t78[0] = vld1q_lane_u32::<3>(t77.add(vgetq_lane_u32::<3>(t76[0]) as usize), t78[0]);
                t78[1] = vld1q_lane_u32::<0>(t77.add(vgetq_lane_u32::<0>(t76[1]) as usize), t78[1]);
                t78[1] = vld1q_lane_u32::<1>(t77.add(vgetq_lane_u32::<1>(t76[1]) as usize), t78[1]);
                t78[1] = vld1q_lane_u32::<2>(t77.add(vgetq_lane_u32::<2>(t76[1]) as usize), t78[1]);
                t78[1] = vld1q_lane_u32::<3>(t77.add(vgetq_lane_u32::<3>(t76[1]) as usize), t78[1]);
                x80 = [vbslq_u32(t74[0], t78[0], x80[0]), vbslq_u32(t74[1], t78[1], x80[1])];
                let t79 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(t74[0], x72[0]), vandq_u32(t74[1], x72[1]))));
                let t80: [uint32x4_t; 2] = [vbslq_u32(t74[0], x72[0], t79), vbslq_u32(t74[1], x72[1], t79)];
                let t81 = arg3 as *const u32;
                let mut t82: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
                t82[0] = vld1q_lane_u32::<0>(t81.add(vgetq_lane_u32::<0>(t80[0]) as usize), t82[0]);
                t82[0] = vld1q_lane_u32::<1>(t81.add(vgetq_lane_u32::<1>(t80[0]) as usize), t82[0]);
                t82[0] = vld1q_lane_u32::<2>(t81.add(vgetq_lane_u32::<2>(t80[0]) as usize), t82[0]);
                t82[0] = vld1q_lane_u32::<3>(t81.add(vgetq_lane_u32::<3>(t80[0]) as usize), t82[0]);
                t82[1] = vld1q_lane_u32::<0>(t81.add(vgetq_lane_u32::<0>(t80[1]) as usize), t82[1]);
                t82[1] = vld1q_lane_u32::<1>(t81.add(vgetq_lane_u32::<1>(t80[1]) as usize), t82[1]);
                t82[1] = vld1q_lane_u32::<2>(t81.add(vgetq_lane_u32::<2>(t80[1]) as usize), t82[1]);
                t82[1] = vld1q_lane_u32::<3>(t81.add(vgetq_lane_u32::<3>(t80[1]) as usize), t82[1]);
                x72 = [vbslq_u32(t74[0], t82[0], x72[0]), vbslq_u32(t74[1], t82[1], x72[1])];
            }
            let t83: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x72[0], t72[0])), vmvnq_u32(vceqq_u32(x72[1], t72[1]))];
            let t84 = [vdupq_n_u32(if ((a1) != (0u32)) { u32::MAX } else { 0 }); 2];
            let t85: [uint32x4_t; 2] = [vandq_u32(t83[0], t84[0]), vandq_u32(t83[1], t84[1])];
            let t86: [uint32x4_t; 2] = [vandq_u32(live[0], t85[0]), vandq_u32(live[1], t85[1])];
            if any(&t86) {
                let t87 = b1_r0r1g_p(l, lid0, t86, arg0, arg1);
                x80 = [vbslq_u32(t86[0], t87[0], x80[0]), vbslq_u32(t86[1], t87[1], x80[1])];
                x72 = [vbslq_u32(t86[0], t72[0], x72[0]), vbslq_u32(t86[1], t72[1], x72[1])];
            }
            let t88 = (arg4 as *mut u32).add((gid0) as usize);
            if all(&live) {
                vst1q_u32(t88.add(0), x80[0]);
                vst1q_u32(t88.add(4), x80[1]);
            } else {
                let mut t89w = 0u32;
                let t89 = core::hint::black_box(&raw mut t89w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t88.add(0), t89).write(vgetq_lane_u32::<0>(x80[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t88.add(1), t89).write(vgetq_lane_u32::<1>(x80[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t88.add(2), t89).write(vgetq_lane_u32::<2>(x80[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t88.add(3), t89).write(vgetq_lane_u32::<3>(x80[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t88.add(4), t89).write(vgetq_lane_u32::<0>(x80[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t88.add(5), t89).write(vgetq_lane_u32::<1>(x80[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t88.add(6), t89).write(vgetq_lane_u32::<2>(x80[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t88.add(7), t89).write(vgetq_lane_u32::<3>(x80[1]));
            }
            let t90 = (arg5 as *mut u32).add((gid0) as usize);
            if all(&live) {
                vst1q_u32(t90.add(0), x72[0]);
                vst1q_u32(t90.add(4), x72[1]);
            } else {
                let mut t91w = 0u32;
                let t91 = core::hint::black_box(&raw mut t91w);
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t90.add(0), t91).write(vgetq_lane_u32::<0>(x72[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t90.add(1), t91).write(vgetq_lane_u32::<1>(x72[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t90.add(2), t91).write(vgetq_lane_u32::<2>(x72[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t90.add(3), t91).write(vgetq_lane_u32::<3>(x72[0]));
                core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t90.add(4), t91).write(vgetq_lane_u32::<0>(x72[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t90.add(5), t91).write(vgetq_lane_u32::<1>(x72[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t90.add(6), t91).write(vgetq_lane_u32::<2>(x72[1]));
                core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t90.add(7), t91).write(vgetq_lane_u32::<3>(x72[1]));
            }
        }
    }
}

#[inline(always)]
unsafe fn b1_r0r1g_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t92 = b2_r0r1g_p(l, lid0, live, arg0, arg1);
    x24 = t92;
    let mut t93 = live;
    loop {
        // Lanes part: each its own rounds.
        let t95 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
        let t96: [uint64x2_t; 4] = [vandq_u64(x24[0], t95[0]), vandq_u64(x24[1], t95[1]), vandq_u64(x24[2], t95[2]), vandq_u64(x24[3], t95[3])];
        let t97 = [vdupq_n_u64(0u64); 4];
        let t98: [uint64x2_t; 4] = [vceqq_u64(t96[0], t97[0]), vceqq_u64(t96[1], t97[1]), vceqq_u64(t96[2], t97[2]), vceqq_u64(t96[3], t97[3])];
        let t99: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t98[0]), vmovn_u64(t98[1])), vcombine_u32(vmovn_u64(t98[2]), vmovn_u64(t98[3]))];
        let t100: [uint32x4_t; 2] = [vmvnq_u32(t99[0]), vmvnq_u32(t99[1])];
        t93 = [vandq_u32(t93[0], t100[0]), vandq_u32(t93[1], t100[1])];
        if !(any(&t93)) { break; }
        let t102: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x24[0]), vmovn_u64(x24[1])), vcombine_u32(vmovn_u64(x24[2]), vmovn_u64(x24[3]))];
        let t103 = b2_r0r1l_p(l, lid0, t93, arg0, arg1, t102);
        x24 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t93[0])))), t103[0], x24[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t93[0])))), t103[1], x24[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t93[1])))), t103[2], x24[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t93[1])))), t103[3], x24[3])];
    }
    let t104: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x24[0]), vmovn_u64(x24[1])), vcombine_u32(vmovn_u64(x24[2]), vmovn_u64(x24[3]))];
    res = t104;
    return res;
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
    let t105: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(gid0), vld1q_u32(LANE[1].as_ptr()))];
    let t106 = [vdupq_n_u32(0u32); 2];
    let t107: [uint32x4_t; 2] = [vceqq_u32(t105[0], t106[0]), vceqq_u32(t105[1], t106[1])];
    let t108: [uint32x4_t; 2] = [vandq_u32(live[0], t107[0]), vandq_u32(live[1], t107[1])];
    if any(&t108) {
        let t109 = [vdupq_n_u64(4294967295u64); 4];
        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t108[0])))), t109[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t108[0])))), t109[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t108[1])))), t109[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t108[1])))), t109[3], res[3])];
        live = [vbicq_u32(live[0], t108[0]), vbicq_u32(live[1], t108[1])];
    }
    if any(&live) {
        let t110 = [vdupq_n_u32(1u32); 2];
        let t111: [uint32x4_t; 2] = [vsubq_u32(t105[0], t110[0]), vsubq_u32(t105[1], t110[1])];
        x24 = [vbslq_u32(live[0], t111[0], x24[0]), vbslq_u32(live[1], t111[1], x24[1])];
        let t112 = b3_r0r1l_p(l, lid0, live, arg0, arg1, x24);
        x32 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t112[0], x32[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t112[1], x32[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t112[2], x32[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t112[3], x32[3])];
        let t113 = [vdupq_n_u64(1u64); 4];
        let t114: [uint64x2_t; 4] = [vandq_u64(x32[0], t113[0]), vandq_u64(x32[1], t113[1]), vandq_u64(x32[2], t113[2]), vandq_u64(x32[3], t113[3])];
        let t115 = [vdupq_n_u64(0u64); 4];
        let t116: [uint64x2_t; 4] = [vceqq_u64(t114[0], t115[0]), vceqq_u64(t114[1], t115[1]), vceqq_u64(t114[2], t115[2]), vceqq_u64(t114[3], t115[3])];
        let t117: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t116[0]), vmovn_u64(t116[1])), vcombine_u32(vmovn_u64(t116[2]), vmovn_u64(t116[3]))];
        let t118: [uint32x4_t; 2] = [vandq_u32(live[0], t117[0]), vandq_u32(live[1], t117[1])];
        if any(&t118) {
            let t119: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x24[0])), vmovl_u32(vget_high_u32(x24[0])), vmovl_u32(vget_low_u32(x24[1])), vmovl_u32(vget_high_u32(x24[1]))];
            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t118[0])))), t119[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t118[0])))), t119[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t118[1])))), t119[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t118[1])))), t119[3], res[3])];
            live = [vbicq_u32(live[0], t118[0]), vbicq_u32(live[1], t118[1])];
        }
        if any(&live) {
            let t120: [uint64x2_t; 4] = [vshlq_u64(x32[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
            let t121: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t120[0]), vmovn_u64(t120[1])), vcombine_u32(vmovn_u64(t120[2]), vmovn_u64(t120[3]))];
            x40 = [vbslq_u32(live[0], t121[0], x40[0]), vbslq_u32(live[1], t121[1], x40[1])];
            let t122: [uint32x4_t; 2] = [vsubq_u32(x40[0], t110[0]), vsubq_u32(x40[1], t110[1])];
            let t123: [uint32x4_t; 2] = [vcgeq_u32(t122[0], t105[0]), vcgeq_u32(t122[1], t105[1])];
            let t124: [uint32x4_t; 2] = [vandq_u32(live[0], t123[0]), vandq_u32(live[1], t123[1])];
            if any(&t124) {
                let t125 = [vdupq_n_u64(4294967295u64); 4];
                res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t124[0])))), t125[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t124[0])))), t125[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t124[1])))), t125[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t124[1])))), t125[3], res[3])];
                live = [vbicq_u32(live[0], t124[0]), vbicq_u32(live[1], t124[1])];
            }
            if any(&live) {
                let t126: [uint32x4_t; 2] = [vsubq_u32(t105[0], x40[0]), vsubq_u32(t105[1], x40[1])];
                x48 = [vbslq_u32(live[0], t126[0], x48[0]), vbslq_u32(live[1], t126[1], x48[1])];
                let t127 = [vdupq_n_u64(2u64); 4];
                let t128: [uint64x2_t; 4] = [vandq_u64(x32[0], t127[0]), vandq_u64(x32[1], t127[1]), vandq_u64(x32[2], t127[2]), vandq_u64(x32[3], t127[3])];
                let t129: [uint64x2_t; 4] = [vceqq_u64(t128[0], t115[0]), vceqq_u64(t128[1], t115[1]), vceqq_u64(t128[2], t115[2]), vceqq_u64(t128[3], t115[3])];
                let t130: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t129[0]), vmovn_u64(t129[1])), vcombine_u32(vmovn_u64(t129[2]), vmovn_u64(t129[3]))];
                let t131: [uint32x4_t; 2] = [vmvnq_u32(t130[0]), vmvnq_u32(t130[1])];
                let t132: [uint32x4_t; 2] = [vandq_u32(live[0], t131[0]), vandq_u32(live[1], t131[1])];
                if any(&t132) {
                    let t133: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                    res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t132[0])))), t133[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t132[0])))), t133[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t132[1])))), t133[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t132[1])))), t133[3], res[3])];
                    live = [vbicq_u32(live[0], t132[0]), vbicq_u32(live[1], t132[1])];
                }
                if any(&live) {
                    let t134: [uint32x4_t; 2] = [vceqq_u32(x48[0], t106[0]), vceqq_u32(x48[1], t106[1])];
                    let t135: [uint32x4_t; 2] = [vandq_u32(live[0], t134[0]), vandq_u32(live[1], t134[1])];
                    if any(&t135) {
                        let t136 = [vdupq_n_u64(4294967295u64); 4];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t135[0])))), t136[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t135[0])))), t136[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t135[1])))), t136[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t135[1])))), t136[3], res[3])];
                        live = [vbicq_u32(live[0], t135[0]), vbicq_u32(live[1], t135[1])];
                    }
                    if any(&live) {
                        let t137 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                        let t138: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                        let t139: [uint64x2_t; 4] = [vorrq_u64(t137[0], t138[0]), vorrq_u64(t137[1], t138[1]), vorrq_u64(t137[2], t138[2]), vorrq_u64(t137[3], t138[3])];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t139[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t139[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t139[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t139[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
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
    let t140 = [vdupq_n_u32(0u32); 2];
    let t141: [uint32x4_t; 2] = [vceqq_u32(x16[0], t140[0]), vceqq_u32(x16[1], t140[1])];
    let t142: [uint32x4_t; 2] = [vandq_u32(live[0], t141[0]), vandq_u32(live[1], t141[1])];
    if any(&t142) {
        let t143 = [vdupq_n_u64(4294967295u64); 4];
        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t142[0])))), t143[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t142[0])))), t143[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t142[1])))), t143[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t142[1])))), t143[3], res[3])];
        live = [vbicq_u32(live[0], t142[0]), vbicq_u32(live[1], t142[1])];
    }
    if any(&live) {
        let t144 = [vdupq_n_u32(1u32); 2];
        let t145: [uint32x4_t; 2] = [vsubq_u32(x16[0], t144[0]), vsubq_u32(x16[1], t144[1])];
        x24 = [vbslq_u32(live[0], t145[0], x24[0]), vbslq_u32(live[1], t145[1], x24[1])];
        let t146 = b3_r0r1l_p(l, lid0, live, arg0, arg1, x24);
        x32 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t146[0], x32[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t146[1], x32[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t146[2], x32[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t146[3], x32[3])];
        let t147 = [vdupq_n_u64(1u64); 4];
        let t148: [uint64x2_t; 4] = [vandq_u64(x32[0], t147[0]), vandq_u64(x32[1], t147[1]), vandq_u64(x32[2], t147[2]), vandq_u64(x32[3], t147[3])];
        let t149 = [vdupq_n_u64(0u64); 4];
        let t150: [uint64x2_t; 4] = [vceqq_u64(t148[0], t149[0]), vceqq_u64(t148[1], t149[1]), vceqq_u64(t148[2], t149[2]), vceqq_u64(t148[3], t149[3])];
        let t151: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t150[0]), vmovn_u64(t150[1])), vcombine_u32(vmovn_u64(t150[2]), vmovn_u64(t150[3]))];
        let t152: [uint32x4_t; 2] = [vandq_u32(live[0], t151[0]), vandq_u32(live[1], t151[1])];
        if any(&t152) {
            let t153: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x24[0])), vmovl_u32(vget_high_u32(x24[0])), vmovl_u32(vget_low_u32(x24[1])), vmovl_u32(vget_high_u32(x24[1]))];
            res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t152[0])))), t153[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t152[0])))), t153[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t152[1])))), t153[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t152[1])))), t153[3], res[3])];
            live = [vbicq_u32(live[0], t152[0]), vbicq_u32(live[1], t152[1])];
        }
        if any(&live) {
            let t154: [uint64x2_t; 4] = [vshlq_u64(x32[0], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[1], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[2], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64))), vshlq_u64(x32[3], vnegq_s64(vdupq_n_s64(((((2u64) as u32)) & 63) as i64)))];
            let t155: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t154[0]), vmovn_u64(t154[1])), vcombine_u32(vmovn_u64(t154[2]), vmovn_u64(t154[3]))];
            x40 = [vbslq_u32(live[0], t155[0], x40[0]), vbslq_u32(live[1], t155[1], x40[1])];
            let t156: [uint32x4_t; 2] = [vsubq_u32(x40[0], t144[0]), vsubq_u32(x40[1], t144[1])];
            let t157: [uint32x4_t; 2] = [vcgeq_u32(t156[0], x16[0]), vcgeq_u32(t156[1], x16[1])];
            let t158: [uint32x4_t; 2] = [vandq_u32(live[0], t157[0]), vandq_u32(live[1], t157[1])];
            if any(&t158) {
                let t159 = [vdupq_n_u64(4294967295u64); 4];
                res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t158[0])))), t159[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t158[0])))), t159[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t158[1])))), t159[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t158[1])))), t159[3], res[3])];
                live = [vbicq_u32(live[0], t158[0]), vbicq_u32(live[1], t158[1])];
            }
            if any(&live) {
                let t160: [uint32x4_t; 2] = [vsubq_u32(x16[0], x40[0]), vsubq_u32(x16[1], x40[1])];
                x48 = [vbslq_u32(live[0], t160[0], x48[0]), vbslq_u32(live[1], t160[1], x48[1])];
                let t161 = [vdupq_n_u64(2u64); 4];
                let t162: [uint64x2_t; 4] = [vandq_u64(x32[0], t161[0]), vandq_u64(x32[1], t161[1]), vandq_u64(x32[2], t161[2]), vandq_u64(x32[3], t161[3])];
                let t163: [uint64x2_t; 4] = [vceqq_u64(t162[0], t149[0]), vceqq_u64(t162[1], t149[1]), vceqq_u64(t162[2], t149[2]), vceqq_u64(t162[3], t149[3])];
                let t164: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t163[0]), vmovn_u64(t163[1])), vcombine_u32(vmovn_u64(t163[2]), vmovn_u64(t163[3]))];
                let t165: [uint32x4_t; 2] = [vmvnq_u32(t164[0]), vmvnq_u32(t164[1])];
                let t166: [uint32x4_t; 2] = [vandq_u32(live[0], t165[0]), vandq_u32(live[1], t165[1])];
                if any(&t166) {
                    let t167: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                    res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t166[0])))), t167[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t166[0])))), t167[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t166[1])))), t167[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t166[1])))), t167[3], res[3])];
                    live = [vbicq_u32(live[0], t166[0]), vbicq_u32(live[1], t166[1])];
                }
                if any(&live) {
                    let t168: [uint32x4_t; 2] = [vceqq_u32(x48[0], t140[0]), vceqq_u32(x48[1], t140[1])];
                    let t169: [uint32x4_t; 2] = [vandq_u32(live[0], t168[0]), vandq_u32(live[1], t168[1])];
                    if any(&t169) {
                        let t170 = [vdupq_n_u64(4294967295u64); 4];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t169[0])))), t170[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t169[0])))), t170[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t169[1])))), t170[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t169[1])))), t170[3], res[3])];
                        live = [vbicq_u32(live[0], t169[0]), vbicq_u32(live[1], t169[1])];
                    }
                    if any(&live) {
                        let t171 = [vdupq_n_u64((1u64).wrapping_shl((32u64) as u32)); 4];
                        let t172: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x48[0])), vmovl_u32(vget_high_u32(x48[0])), vmovl_u32(vget_low_u32(x48[1])), vmovl_u32(vget_high_u32(x48[1]))];
                        let t173: [uint64x2_t; 4] = [vorrq_u64(t171[0], t172[0]), vorrq_u64(t171[1], t172[1]), vorrq_u64(t171[2], t172[2]), vorrq_u64(t171[3], t172[3])];
                        res = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0])))), t173[0], res[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0])))), t173[1], res[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1])))), t173[2], res[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))), t173[3], res[3])];
                        live = [vbicq_u32(live[0], live[0]), vbicq_u32(live[1], live[1])];
                    }
                }
            }
        }
    }
    res
}

#[inline(always)]
unsafe fn b3_r0r1l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t174 = b4_r0r1sl_p(l, lid0, live, arg0, arg1, 0u32, x16);
    res = t174;
    return res;
}

#[inline(always)]
unsafe fn b4_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: u32 = 0;
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: u64 = 0;
    let mut x56: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t175 = b5_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    x32 = t175;
    let t176: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    let t177 = b6_r0r1sl_p(l, lid0, live, arg0, arg1, (1u32).wrapping_add(x16), t176);
    x40 = t177;
    let t178 = b7_s_p(l, lid0, live, x32);
    x48 = t178;
    let t179 = [vdupq_n_u32((1024u32).wrapping_sub(1u32)); 2];
    let t180: [uint32x4_t; 2] = [vandq_u32(x24[0], t179[0]), vandq_u32(x24[1], t179[1])];
    let t181 = [vdupq_n_u32(x32); 2];
    let t182: [uint32x4_t; 2] = [vmulq_u32(t180[0], t181[0]), vmulq_u32(t180[1], t181[1])];
    let t183 = b8_r0r1lls_p(l, lid0, live, arg0, arg1, x40, t182, 8u32);
    let t184 = [vdupq_n_u64(x48); 4];
    let t185: [uint64x2_t; 4] = [vandq_u64(t183[0], t184[0]), vandq_u64(t183[1], t184[1]), vandq_u64(t183[2], t184[2]), vandq_u64(t183[3], t184[3])];
    x56 = t185;
    let t186 = [vdupq_n_u64(x48); 4];
    let t187: [uint64x2_t; 4] = [vceqq_u64(x56[0], t186[0]), vceqq_u64(x56[1], t186[1]), vceqq_u64(x56[2], t186[2]), vceqq_u64(x56[3], t186[3])];
    let t188: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t187[0]), vmovn_u64(t187[1])), vcombine_u32(vmovn_u64(t187[2]), vmovn_u64(t187[3]))];
    let mut t189: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t190: [uint32x4_t; 2] = [vandq_u32(live[0], t188[0]), vandq_u32(live[1], t188[1])];
    let t191: [uint32x4_t; 2] = [vbicq_u32(live[0], t188[0]), vbicq_u32(live[1], t188[1])];
    if any(&t190) {
        let t192 = b9_r0r1sl_p(l, lid0, t190, arg0, arg1, (x16).wrapping_add(1u32), x24);
        t189 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t190[0])))), t192[0], t189[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t190[0])))), t192[1], t189[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t190[1])))), t192[2], t189[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t190[1])))), t192[3], t189[3])];
    }
    {
        t189 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t191[0])))), x56[0], t189[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t191[0])))), x56[1], t189[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t191[1])))), x56[2], t189[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t191[1])))), x56[3], t189[3])];
    }
    res = t189;
    return res;
}

#[inline(always)]
unsafe fn b5_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t193 = b10_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    res = (((t193).wrapping_shr(((4u32).wrapping_mul(x16)) as u32)) & (15u32));
    return res;
}

#[inline(always)]
unsafe fn b6_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t194 = b10_r0r1s_p(l, lid0, live, arg0, arg1, 40u32);
    let t195 = [vdupq_n_u32(t194); 2];
    let t196: [uint32x4_t; 2] = [vmulq_u32(x24[0], t195[0]), vmulq_u32(x24[1], t195[1])];
    let t197 = b11_r0r1s_p(l, lid0, live, arg0, arg1, 0u32);
    let t198 = [vdupq_n_u32(t197); 2];
    let t199: [uint32x4_t; 2] = [vaddq_u32(t196[0], t198[0]), vaddq_u32(t196[1], t198[1])];
    x32 = t199;
    if ((x16) == (0u32)) {
        res = x32;
        return res;
    }
    let t200 = b10_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    let t201: [uint32x4_t; 2] = [vshlq_u32(x24[0], vdupq_n_s32(((10u32) & 31) as i32)), vshlq_u32(x24[1], vdupq_n_s32(((10u32) & 31) as i32))];
    let t202 = [vdupq_n_u32(t200); 2];
    let t203: [uint32x4_t; 2] = [vsubq_u32(t202[0], t201[0]), vsubq_u32(t202[1], t201[1])];
    let t204 = b12_ls_p(l, lid0, live, t203, 1024u32);
    x40 = t204;
    let t205 = b11_r0r1s_p(l, lid0, live, arg0, arg1, x16);
    let t206 = [vdupq_n_u32(t205); 2];
    let t207: [uint32x4_t; 2] = [vmulq_u32(x40[0], t206[0]), vmulq_u32(x40[1], t206[1])];
    let t208: [uint32x4_t; 2] = [vaddq_u32(x32[0], t207[0]), vaddq_u32(x32[1], t207[1])];
    res = t208;
    return res;
}

#[inline(always)]
unsafe fn b7_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t209: u64;
    if ((x0) >= (8u32)) {
        t209 = !(0u64);
    } else {
        t209 = ((1u64).wrapping_shl(((((x0).wrapping_mul(8u32)) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t209;
    return res;
}

#[inline(always)]
unsafe fn b7_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t210 = [vdupq_n_u32(8u32); 2];
    let t211: [uint32x4_t; 2] = [vcgeq_u32(x0[0], t210[0]), vcgeq_u32(x0[1], t210[1])];
    let mut t212: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t213: [uint32x4_t; 2] = [vandq_u32(live[0], t211[0]), vandq_u32(live[1], t211[1])];
    let t214: [uint32x4_t; 2] = [vbicq_u32(live[0], t211[0]), vbicq_u32(live[1], t211[1])];
    {
        let t215 = [vdupq_n_u64(!(0u64)); 4];
        t212 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t213[0])))), t215[0], t212[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t213[0])))), t215[1], t212[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t213[1])))), t215[2], t212[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t213[1])))), t215[3], t212[3])];
    }
    {
        let t216: [uint32x4_t; 2] = [vmulq_u32(x0[0], t210[0]), vmulq_u32(x0[1], t210[1])];
        let t217 = [vdupq_n_u64(1u64); 4];
        let t218: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t216[0])), vmovl_u32(vget_high_u32(t216[0])), vmovl_u32(vget_low_u32(t216[1])), vmovl_u32(vget_high_u32(t216[1]))];
        let t219: [uint64x2_t; 4] = [vshlq_u64(t217[0], vreinterpretq_s64_u64(vandq_u64(t218[0], vdupq_n_u64(63)))), vshlq_u64(t217[1], vreinterpretq_s64_u64(vandq_u64(t218[1], vdupq_n_u64(63)))), vshlq_u64(t217[2], vreinterpretq_s64_u64(vandq_u64(t218[2], vdupq_n_u64(63)))), vshlq_u64(t217[3], vreinterpretq_s64_u64(vandq_u64(t218[3], vdupq_n_u64(63))))];
        let t220: [uint64x2_t; 4] = [vsubq_u64(t219[0], t217[0]), vsubq_u64(t219[1], t217[1]), vsubq_u64(t219[2], t217[2]), vsubq_u64(t219[3], t217[3])];
        t212 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t214[0])))), t220[0], t212[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t214[0])))), t220[1], t212[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t214[1])))), t220[2], t212[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t214[1])))), t220[3], t212[3])];
    }
    res = t212;
    return res;
}

#[inline(always)]
unsafe fn b8_r0r1lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: [uint32x4_t; 2], x24: [uint32x4_t; 2], x32: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t221 = b13_r0lls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t221;
    return res;
}

#[inline(always)]
unsafe fn b9_r0r1sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2]) -> [uint64x2_t; 4] {
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
    let t222 = b10_r0r1s_p(l, lid0, live, arg0, arg1, (44u32).wrapping_add((4u32).wrapping_mul(x16)));
    x32 = t222;
    let t223 = b10_r0r1s_p(l, lid0, live, arg0, arg1, (60u32).wrapping_add((4u32).wrapping_mul(x16)));
    x40 = t223;
    let t224: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((10u32) & 31) as i32)))];
    x48 = t224;
    let t225 = [vdupq_n_u32(4u32); 2];
    let t226: [uint32x4_t; 2] = [vmulq_u32(x48[0], t225[0]), vmulq_u32(x48[1], t225[1])];
    let t227 = b14_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t226, 4u32);
    let t228: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t227[0]), vmovn_u64(t227[1])), vcombine_u32(vmovn_u64(t227[2]), vmovn_u64(t227[3]))];
    x56 = t228;
    let t229: [uint32x4_t; 2] = [vmulq_u32(x48[0], t225[0]), vmulq_u32(x48[1], t225[1])];
    let t230: [uint32x4_t; 2] = [vaddq_u32(t229[0], t225[0]), vaddq_u32(t229[1], t225[1])];
    let t231 = b14_r0r1sls_p(l, lid0, live, arg0, arg1, x40, t230, 4u32);
    let t232: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t231[0]), vmovn_u64(t231[1])), vcombine_u32(vmovn_u64(t231[2]), vmovn_u64(t231[3]))];
    x64 = t232;
    let t233 = b10_r0r1s_p(l, lid0, live, arg0, arg1, 4u32);
    x72 = ((t233).wrapping_add((256u32).wrapping_sub(1u32))).wrapping_shr((8u32) as u32);
    let t234: [uint32x4_t; 2] = [vmulq_u32(x48[0], t225[0]), vmulq_u32(x48[1], t225[1])];
    let t235 = [vdupq_n_u32(x72); 2];
    let t236: [uint32x4_t; 2] = [vsubq_u32(t235[0], t234[0]), vsubq_u32(t235[1], t234[1])];
    let t237 = b15_sl_p(l, lid0, live, 4u32, t236);
    x80 = t237;
    let t238: [uint32x4_t; 2] = [vshlq_u32(x24[0], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32))), vshlq_u32(x24[1], vnegq_s32(vdupq_n_s32(((8u32) & 31) as i32)))];
    let t239 = [vdupq_n_u32((4u32).wrapping_sub(1u32)); 2];
    let t240: [uint32x4_t; 2] = [vandq_u32(t238[0], t239[0]), vandq_u32(t238[1], t239[1])];
    x88 = t240;
    let t241 = [vdupq_n_u32(0u32); 2];
    let t242: [uint32x4_t; 2] = [vceqq_u32(x88[0], t241[0]), vceqq_u32(x88[1], t241[1])];
    let mut t243: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t244: [uint32x4_t; 2] = [vandq_u32(live[0], t242[0]), vandq_u32(live[1], t242[1])];
    let t245: [uint32x4_t; 2] = [vbicq_u32(live[0], t242[0]), vbicq_u32(live[1], t242[1])];
    {
        t243 = [vbslq_u32(t244[0], t241[0], t243[0]), vbslq_u32(t244[1], t241[1], t243[1])];
    }
    if any(&t245) {
        let t246 = [vdupq_n_u32(1u32); 2];
        let t247: [uint32x4_t; 2] = [vsubq_u32(x88[0], t246[0]), vsubq_u32(x88[1], t246[1])];
        let t248 = [vdupq_n_u32(2u32); 2];
        let t249: [uint32x4_t; 2] = [vmulq_u32(t248[0], t247[0]), vmulq_u32(t248[1], t247[1])];
        let t250: [uint32x4_t; 2] = [vaddq_u32(x56[0], t249[0]), vaddq_u32(x56[1], t249[1])];
        let t251 = b16_ll_p(l, lid0, t245, t250, x64);
        let t252 = b14_r0r1sls_p(l, lid0, t245, arg0, arg1, x32, t251, 2u32);
        let t253: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t252[0]), vmovn_u64(t252[1])), vcombine_u32(vmovn_u64(t252[2]), vmovn_u64(t252[3]))];
        t243 = [vbslq_u32(t245[0], t253[0], t243[0]), vbslq_u32(t245[1], t253[1], t243[1])];
    }
    x96 = t243;
    let t254 = [vdupq_n_u32(2u32); 2];
    let t255: [uint32x4_t; 2] = [vmulq_u32(t254[0], x88[0]), vmulq_u32(t254[1], x88[1])];
    let t256: [uint32x4_t; 2] = [vaddq_u32(x56[0], t255[0]), vaddq_u32(x56[1], t255[1])];
    let t257 = b16_ll_p(l, lid0, live, t256, x64);
    let t258 = b14_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t257, 2u32);
    let t259: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t258[0]), vmovn_u64(t258[1])), vcombine_u32(vmovn_u64(t258[2]), vmovn_u64(t258[3]))];
    x104 = t259;
    let t260: [uint32x4_t; 2] = [vmulq_u32(t254[0], x80[0]), vmulq_u32(t254[1], x80[1])];
    let t261: [uint32x4_t; 2] = [vaddq_u32(x56[0], t260[0]), vaddq_u32(x56[1], t260[1])];
    x112 = t261;
    let t262 = [vdupq_n_u32((256u32).wrapping_sub(1u32)); 2];
    let t263: [uint32x4_t; 2] = [vandq_u32(x24[0], t262[0]), vandq_u32(x24[1], t262[1])];
    x120 = t263;
    x128 = 9259542123273814144u64;
    let t264: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x120[0])), vmovl_u32(vget_high_u32(x120[0])), vmovl_u32(vget_low_u32(x120[1])), vmovl_u32(vget_high_u32(x120[1]))];
    let t265 = [vdupq_n_u64(72340172838076673u64); 4];
    let mut t266: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    t266[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t264[0]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t265[0]) as u64) as u64, t266[0]);
    t266[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t264[0]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t265[0]) as u64) as u64, t266[0]);
    t266[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t264[1]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t265[1]) as u64) as u64, t266[1]);
    t266[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t264[1]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t265[1]) as u64) as u64, t266[1]);
    t266[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t264[2]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t265[2]) as u64) as u64, t266[2]);
    t266[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t264[2]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t265[2]) as u64) as u64, t266[2]);
    t266[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t264[3]) as u64).wrapping_mul(vgetq_lane_u64::<0>(t265[3]) as u64) as u64, t266[3]);
    t266[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t264[3]) as u64).wrapping_mul(vgetq_lane_u64::<1>(t265[3]) as u64) as u64, t266[3]);
    x136 = t266;
    x144 = x96;
    x152 = x96;
    let mut t267 = live;
    loop {
        // Lanes part: each its own rounds.
        let t269: [uint32x4_t; 2] = [vcltq_u32(x152[0], x104[0]), vcltq_u32(x152[1], x104[1])];
        t267 = [vandq_u32(t267[0], t269[0]), vandq_u32(t267[1], t269[1])];
        if !(any(&t267)) { break; }
        let t271: [uint32x4_t; 2] = [vaddq_u32(x112[0], x152[0]), vaddq_u32(x112[1], x152[1])];
        let t272 = b16_ll_p(l, lid0, t267, t271, x64);
        let t273 = b14_r0r1sls_p(l, lid0, t267, arg0, arg1, x32, t272, 8u32);
        x160 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t267[0])))), t273[0], x160[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t267[0])))), t273[1], x160[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t267[1])))), t273[2], x160[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t267[1])))), t273[3], x160[3])];
        let t274 = [vdupq_n_u64(x128); 4];
        let t275: [uint64x2_t; 4] = [vorrq_u64(x160[0], t274[0]), vorrq_u64(x160[1], t274[1]), vorrq_u64(x160[2], t274[2]), vorrq_u64(x160[3], t274[3])];
        let t276 = [vdupq_n_u64(!(x128)); 4];
        let t277: [uint64x2_t; 4] = [vandq_u64(x136[0], t276[0]), vandq_u64(x136[1], t276[1]), vandq_u64(x136[2], t276[2]), vandq_u64(x136[3], t276[3])];
        let t278: [uint64x2_t; 4] = [vsubq_u64(t275[0], t277[0]), vsubq_u64(t275[1], t277[1]), vsubq_u64(t275[2], t277[2]), vsubq_u64(t275[3], t277[3])];
        x168 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t267[0])))), t278[0], x168[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t267[0])))), t278[1], x168[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t267[1])))), t278[2], x168[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t267[1])))), t278[3], x168[3])];
        let t279: [uint64x2_t; 4] = [vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[0]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[1]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[2]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x160[3])))];
        let t280: [uint64x2_t; 4] = [vandq_u64(t279[0], x136[0]), vandq_u64(t279[1], x136[1]), vandq_u64(t279[2], x136[2]), vandq_u64(t279[3], x136[3])];
        let t281: [uint64x2_t; 4] = [veorq_u64(x160[0], x136[0]), veorq_u64(x160[1], x136[1]), veorq_u64(x160[2], x136[2]), veorq_u64(x160[3], x136[3])];
        let t282: [uint64x2_t; 4] = [vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t281[0]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t281[1]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t281[2]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(t281[3])))];
        let t283: [uint64x2_t; 4] = [vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[0]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[1]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[2]))), vreinterpretq_u64_u32(vmvnq_u32(vreinterpretq_u32_u64(x168[3])))];
        let t284: [uint64x2_t; 4] = [vandq_u64(t282[0], t283[0]), vandq_u64(t282[1], t283[1]), vandq_u64(t282[2], t283[2]), vandq_u64(t282[3], t283[3])];
        let t285: [uint64x2_t; 4] = [vorrq_u64(t280[0], t284[0]), vorrq_u64(t280[1], t284[1]), vorrq_u64(t280[2], t284[2]), vorrq_u64(t280[3], t284[3])];
        let t286 = [vdupq_n_u64(x128); 4];
        let t287: [uint64x2_t; 4] = [vandq_u64(t286[0], t285[0]), vandq_u64(t286[1], t285[1]), vandq_u64(t286[2], t285[2]), vandq_u64(t286[3], t285[3])];
        x176 = t287;
        let t288: [uint32x4_t; 2] = [vsubq_u32(x104[0], x152[0]), vsubq_u32(x104[1], x152[1])];
        x184 = t288;
        let t289 = [vdupq_n_u32(8u32); 2];
        let t290: [uint32x4_t; 2] = [vcgeq_u32(x184[0], t289[0]), vcgeq_u32(x184[1], t289[1])];
        let mut t291: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
        let t292: [uint32x4_t; 2] = [vandq_u32(t267[0], t290[0]), vandq_u32(t267[1], t290[1])];
        let t293: [uint32x4_t; 2] = [vbicq_u32(t267[0], t290[0]), vbicq_u32(t267[1], t290[1])];
        {
            t291 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t292[0])))), x176[0], t291[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t292[0])))), x176[1], t291[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t292[1])))), x176[2], t291[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t292[1])))), x176[3], t291[3])];
        }
        {
            let t294: [uint32x4_t; 2] = [vmulq_u32(t289[0], x184[0]), vmulq_u32(t289[1], x184[1])];
            let t295 = [vdupq_n_u64(1u64); 4];
            let t296: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t294[0])), vmovl_u32(vget_high_u32(t294[0])), vmovl_u32(vget_low_u32(t294[1])), vmovl_u32(vget_high_u32(t294[1]))];
            let t297: [uint64x2_t; 4] = [vshlq_u64(t295[0], vreinterpretq_s64_u64(vandq_u64(t296[0], vdupq_n_u64(63)))), vshlq_u64(t295[1], vreinterpretq_s64_u64(vandq_u64(t296[1], vdupq_n_u64(63)))), vshlq_u64(t295[2], vreinterpretq_s64_u64(vandq_u64(t296[2], vdupq_n_u64(63)))), vshlq_u64(t295[3], vreinterpretq_s64_u64(vandq_u64(t296[3], vdupq_n_u64(63))))];
            let t298: [uint64x2_t; 4] = [vsubq_u64(t297[0], t295[0]), vsubq_u64(t297[1], t295[1]), vsubq_u64(t297[2], t295[2]), vsubq_u64(t297[3], t295[3])];
            let t299: [uint64x2_t; 4] = [vandq_u64(x176[0], t298[0]), vandq_u64(x176[1], t298[1]), vandq_u64(x176[2], t298[2]), vandq_u64(x176[3], t298[3])];
            t291 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t293[0])))), t299[0], t291[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t293[0])))), t299[1], t291[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t293[1])))), t299[2], t291[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t293[1])))), t299[3], t291[3])];
        }
        let t300 = b17_l_p(l, lid0, t267, t291);
        let t301: [uint32x4_t; 2] = [vaddq_u32(x144[0], t300[0]), vaddq_u32(x144[1], t300[1])];
        x144 = [vbslq_u32(t267[0], t301[0], x144[0]), vbslq_u32(t267[1], t301[1], x144[1])];
        let t302: [uint32x4_t; 2] = [vandq_u32(t267[0], live[0]), vandq_u32(t267[1], live[1])];
        x152 = [vaddq_u32(x152[0], vandq_u32(t302[0], vdupq_n_u32(8))), vaddq_u32(x152[1], vandq_u32(t302[1], vdupq_n_u32(8)))];
    }
    x96 = x144;
    let t303 = b10_r0r1s_p(l, lid0, live, arg0, arg1, 16u32);
    let t304 = [vdupq_n_u32((((t303).wrapping_shr((((((4u32).wrapping_mul(x16)).wrapping_add(16u32)) & (31u32))) as u32)) & (15u32))); 2];
    x152 = t304;
    let t305 = [vdupq_n_u32(1u32); 2];
    let t306: [uint32x4_t; 2] = [vsubq_u32(x80[0], t305[0]), vsubq_u32(x80[1], t305[1])];
    let t307: [uint32x4_t; 2] = [vmulq_u32(t254[0], t306[0]), vmulq_u32(t254[1], t306[1])];
    let t308: [uint32x4_t; 2] = [vaddq_u32(x56[0], t307[0]), vaddq_u32(x56[1], t307[1])];
    let t309 = b16_ll_p(l, lid0, live, t308, x64);
    let t310 = b14_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t309, 2u32);
    let t311: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t310[0]), vmovn_u64(t310[1])), vcombine_u32(vmovn_u64(t310[2]), vmovn_u64(t310[3]))];
    let t312: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t311[0])), vmovl_u32(vget_high_u32(t311[0])), vmovl_u32(vget_low_u32(t311[1])), vmovl_u32(vget_high_u32(t311[1]))];
    x160 = t312;
    let t313: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x160[0]), vmovn_u64(x160[1])), vcombine_u32(vmovn_u64(x160[2]), vmovn_u64(x160[3]))];
    let t314: [uint32x4_t; 2] = [vaddq_u32(x112[0], t313[0]), vaddq_u32(x112[1], t313[1])];
    let t315: [uint32x4_t; 2] = [vmulq_u32(x96[0], x152[0]), vmulq_u32(x96[1], x152[1])];
    let t316: [uint32x4_t; 2] = [vaddq_u32(t314[0], t315[0]), vaddq_u32(t314[1], t315[1])];
    let t317 = b16_ll_p(l, lid0, live, t316, x64);
    let t318: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t317[0])), vmovl_u32(vget_high_u32(t317[0])), vmovl_u32(vget_low_u32(t317[1])), vmovl_u32(vget_high_u32(t317[1]))];
    x168 = t318;
    let t319: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(x168[0]), vmovn_u64(x168[1])), vcombine_u32(vmovn_u64(x168[2]), vmovn_u64(x168[3]))];
    let t320 = b14_r0r1sls_p(l, lid0, live, arg0, arg1, x32, t319, 8u32);
    let t321 = b7_l_p(l, lid0, live, x152);
    let t322: [uint64x2_t; 4] = [vandq_u64(t320[0], t321[0]), vandq_u64(t320[1], t321[1]), vandq_u64(t320[2], t321[2]), vandq_u64(t320[3], t321[3])];
    res = t322;
    return res;
}

#[inline(always)]
unsafe fn b10_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    res = (arg1.wrapping_add(((((x16).checked_div(4u32).unwrap_or(0)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned();
    return res;
}

#[inline(always)]
unsafe fn b11_r0r1s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32) -> u32 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u32 = 0;
    let t323 = b10_r0r1s_p(l, lid0, live, arg0, arg1, (20u32).wrapping_add((4u32).wrapping_mul(x16)));
    res = t323;
    return res;
}

#[inline(always)]
unsafe fn b12_ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: u32) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t324 = [vdupq_n_u32(x8); 2];
    let t325: [uint32x4_t; 2] = [vcltq_u32(x0[0], t324[0]), vcltq_u32(x0[1], t324[1])];
    let mut t326: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t327: [uint32x4_t; 2] = [vandq_u32(live[0], t325[0]), vandq_u32(live[1], t325[1])];
    let t328: [uint32x4_t; 2] = [vbicq_u32(live[0], t325[0]), vbicq_u32(live[1], t325[1])];
    {
        t326 = [vbslq_u32(t327[0], x0[0], t326[0]), vbslq_u32(t327[1], x0[1], t326[1])];
    }
    {
        let t329 = [vdupq_n_u32(x8); 2];
        t326 = [vbslq_u32(t328[0], t329[0], t326[0]), vbslq_u32(t328[1], t329[1], t326[1])];
    }
    res = t326;
    return res;
}

#[inline(always)]
unsafe fn b13_r0lls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint32x4_t; 2], x16: [uint32x4_t; 2], x24: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t330: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x8[0])), vmovl_u32(vget_high_u32(x8[0])), vmovl_u32(vget_low_u32(x8[1])), vmovl_u32(vget_high_u32(x8[1]))];
    let t331: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x16[0])), vmovl_u32(vget_high_u32(x16[0])), vmovl_u32(vget_low_u32(x16[1])), vmovl_u32(vget_high_u32(x16[1]))];
    let t332: [uint64x2_t; 4] = [vaddq_u64(t330[0], t331[0]), vaddq_u64(t330[1], t331[1]), vaddq_u64(t330[2], t331[2]), vaddq_u64(t330[3], t331[3])];
    let t333 = b18_r0ls_p(l, lid0, live, arg0, t332, x24);
    x32 = t333;
    let t334: [uint64x2_t; 4];
    if ((x24) >= (8u32)) {
        t334 = x32;
    } else {
        let t335 = b19_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t336 = [vdupq_n_u64(t335); 4];
        let t337: [uint64x2_t; 4] = [vandq_u64(x32[0], t336[0]), vandq_u64(x32[1], t336[1]), vandq_u64(x32[2], t336[2]), vandq_u64(x32[3], t336[3])];
        t334 = t337;
    }
    res = t334;
    return res;
}

#[inline(always)]
unsafe fn b14_r0r1sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, arg1: *const u8, x16: u32, x24: [uint32x4_t; 2], x32: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t338 = b20_r0sls_p(l, lid0, live, arg0, x16, x24, x32);
    res = t338;
    return res;
}

#[inline(always)]
unsafe fn b15_sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32, x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t339 = [vdupq_n_u32(x0); 2];
    let t340: [uint32x4_t; 2] = [vcltq_u32(t339[0], x8[0]), vcltq_u32(t339[1], x8[1])];
    let mut t341: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t342: [uint32x4_t; 2] = [vandq_u32(live[0], t340[0]), vandq_u32(live[1], t340[1])];
    let t343: [uint32x4_t; 2] = [vbicq_u32(live[0], t340[0]), vbicq_u32(live[1], t340[1])];
    {
        let t344 = [vdupq_n_u32(x0); 2];
        t341 = [vbslq_u32(t342[0], t344[0], t341[0]), vbslq_u32(t342[1], t344[1], t341[1])];
    }
    {
        t341 = [vbslq_u32(t343[0], x8[0], t341[0]), vbslq_u32(t343[1], x8[1], t341[1])];
    }
    res = t341;
    return res;
}

#[inline(always)]
unsafe fn b16_ll_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint32x4_t; 2], x8: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t345: [uint32x4_t; 2] = [vcltq_u32(x0[0], x8[0]), vcltq_u32(x0[1], x8[1])];
    let mut t346: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t347: [uint32x4_t; 2] = [vandq_u32(live[0], t345[0]), vandq_u32(live[1], t345[1])];
    let t348: [uint32x4_t; 2] = [vbicq_u32(live[0], t345[0]), vbicq_u32(live[1], t345[1])];
    {
        t346 = [vbslq_u32(t347[0], x0[0], t346[0]), vbslq_u32(t347[1], x0[1], t346[1])];
    }
    {
        t346 = [vbslq_u32(t348[0], x8[0], t346[0]), vbslq_u32(t348[1], x8[1], t346[1])];
    }
    res = t346;
    return res;
}

#[inline(always)]
unsafe fn b17_l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: [uint64x2_t; 4]) -> [uint32x4_t; 2] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t349: [uint64x2_t; 4] = [vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[0]))))), vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[1]))))), vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[2]))))), vpaddlq_u32(vpaddlq_u16(vpaddlq_u8(vcntq_u8(vreinterpretq_u8_u64(x0[3])))))];
    let t350: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t349[0]), vmovn_u64(t349[1])), vcombine_u32(vmovn_u64(t349[2]), vmovn_u64(t349[3]))];
    res = t350;
    return res;
}

#[inline(always)]
unsafe fn b18_r0ls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: [uint64x2_t; 4], x16: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x24: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut x32: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x40: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t351 = [vdupq_n_u64(!(7u64)); 4];
    let t352: [uint64x2_t; 4] = [vandq_u64(x8[0], t351[0]), vandq_u64(x8[1], t351[1]), vandq_u64(x8[2], t351[2]), vandq_u64(x8[3], t351[3])];
    let t353: [uint64x2_t; 4] = [vandq_u64(t352[0], vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[0]))))), vandq_u64(t352[1], vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[0]))))), vandq_u64(t352[2], vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(live[1]))))), vandq_u64(t352[3], vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(live[1])))))];
    let t354: [uint64x2_t; 4] = [vaddq_u64(vdupq_n_u64((arg0) as u64), t353[0]), vaddq_u64(vdupq_n_u64((arg0) as u64), t353[1]), vaddq_u64(vdupq_n_u64((arg0) as u64), t353[2]), vaddq_u64(vdupq_n_u64((arg0) as u64), t353[3])];
    x24 = t354;
    let t355 = [vdupq_n_u64(7u64); 4];
    let t356: [uint64x2_t; 4] = [vandq_u64(x8[0], t355[0]), vandq_u64(x8[1], t355[1]), vandq_u64(x8[2], t355[2]), vandq_u64(x8[3], t355[3])];
    let t357: [uint32x4_t; 2] = [vcombine_u32(vmovn_u64(t356[0]), vmovn_u64(t356[1])), vcombine_u32(vmovn_u64(t356[2]), vmovn_u64(t356[3]))];
    let t358 = [vdupq_n_u32(8u32); 2];
    let t359: [uint32x4_t; 2] = [vmulq_u32(t357[0], t358[0]), vmulq_u32(t357[1], t358[1])];
    x32 = t359;
    let mut t360: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    t360[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[0]) as usize as *const u64).read_unaligned(), t360[0]);
    t360[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[0]) as usize as *const u64).read_unaligned(), t360[0]);
    t360[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[1]) as usize as *const u64).read_unaligned(), t360[1]);
    t360[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[1]) as usize as *const u64).read_unaligned(), t360[1]);
    t360[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[2]) as usize as *const u64).read_unaligned(), t360[2]);
    t360[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[2]) as usize as *const u64).read_unaligned(), t360[2]);
    t360[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(x24[3]) as usize as *const u64).read_unaligned(), t360[3]);
    t360[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(x24[3]) as usize as *const u64).read_unaligned(), t360[3]);
    let t361: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x32[0])), vmovl_u32(vget_high_u32(x32[0])), vmovl_u32(vget_low_u32(x32[1])), vmovl_u32(vget_high_u32(x32[1]))];
    let t362: [uint64x2_t; 4] = [vshlq_u64(t360[0], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t361[0], vdupq_n_u64(63))))), vshlq_u64(t360[1], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t361[1], vdupq_n_u64(63))))), vshlq_u64(t360[2], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t361[2], vdupq_n_u64(63))))), vshlq_u64(t360[3], vnegq_s64(vreinterpretq_s64_u64(vandq_u64(t361[3], vdupq_n_u64(63)))))];
    x40 = t362;
    let t363 = [vdupq_n_u32(0u32); 2];
    let t364: [uint32x4_t; 2] = [vmvnq_u32(vceqq_u32(x32[0], t363[0])), vmvnq_u32(vceqq_u32(x32[1], t363[1]))];
    let t365 = [vdupq_n_u32((x16).wrapping_mul(8u32)); 2];
    let t366: [uint32x4_t; 2] = [vaddq_u32(x32[0], t365[0]), vaddq_u32(x32[1], t365[1])];
    let t367 = [vdupq_n_u32(64u32); 2];
    let t368: [uint32x4_t; 2] = [vcgtq_u32(t366[0], t367[0]), vcgtq_u32(t366[1], t367[1])];
    let t369: [uint32x4_t; 2] = [vandq_u32(t364[0], t368[0]), vandq_u32(t364[1], t368[1])];
    let mut t370: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t371: [uint32x4_t; 2] = [vandq_u32(live[0], t369[0]), vandq_u32(live[1], t369[1])];
    let t372: [uint32x4_t; 2] = [vbicq_u32(live[0], t369[0]), vbicq_u32(live[1], t369[1])];
    if any(&t371) {
        let t373 = [vdupq_n_u64((1u64).wrapping_mul(8u64)); 4];
        let t374: [uint64x2_t; 4] = [vaddq_u64(x24[0], t373[0]), vaddq_u64(x24[1], t373[1]), vaddq_u64(x24[2], t373[2]), vaddq_u64(x24[3], t373[3])];
        let mut t375: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
        t375[0] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t374[0]) as usize as *const u64).read_unaligned(), t375[0]);
        t375[0] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t374[0]) as usize as *const u64).read_unaligned(), t375[0]);
        t375[1] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t374[1]) as usize as *const u64).read_unaligned(), t375[1]);
        t375[1] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t374[1]) as usize as *const u64).read_unaligned(), t375[1]);
        t375[2] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t374[2]) as usize as *const u64).read_unaligned(), t375[2]);
        t375[2] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t374[2]) as usize as *const u64).read_unaligned(), t375[2]);
        t375[3] = vsetq_lane_u64::<0>((vgetq_lane_u64::<0>(t374[3]) as usize as *const u64).read_unaligned(), t375[3]);
        t375[3] = vsetq_lane_u64::<1>((vgetq_lane_u64::<1>(t374[3]) as usize as *const u64).read_unaligned(), t375[3]);
        let t376: [uint32x4_t; 2] = [vsubq_u32(t367[0], x32[0]), vsubq_u32(t367[1], x32[1])];
        let t377: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(t376[0])), vmovl_u32(vget_high_u32(t376[0])), vmovl_u32(vget_low_u32(t376[1])), vmovl_u32(vget_high_u32(t376[1]))];
        let t378: [uint64x2_t; 4] = [vshlq_u64(t375[0], vreinterpretq_s64_u64(vandq_u64(t377[0], vdupq_n_u64(63)))), vshlq_u64(t375[1], vreinterpretq_s64_u64(vandq_u64(t377[1], vdupq_n_u64(63)))), vshlq_u64(t375[2], vreinterpretq_s64_u64(vandq_u64(t377[2], vdupq_n_u64(63)))), vshlq_u64(t375[3], vreinterpretq_s64_u64(vandq_u64(t377[3], vdupq_n_u64(63))))];
        let t379: [uint64x2_t; 4] = [vorrq_u64(x40[0], t378[0]), vorrq_u64(x40[1], t378[1]), vorrq_u64(x40[2], t378[2]), vorrq_u64(x40[3], t378[3])];
        t370 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t371[0])))), t379[0], t370[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t371[0])))), t379[1], t370[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t371[1])))), t379[2], t370[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t371[1])))), t379[3], t370[3])];
    }
    {
        t370 = [vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t372[0])))), x40[0], t370[0]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t372[0])))), x40[1], t370[1]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_low_s32(vreinterpretq_s32_u32(t372[1])))), x40[2], t370[2]), vbslq_u64(vreinterpretq_u64_s64(vmovl_s32(vget_high_s32(vreinterpretq_s32_u32(t372[1])))), x40[3], t370[3])];
    }
    res = t370;
    return res;
}

#[inline(always)]
unsafe fn b19_s_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: u32) -> u64 {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut live = k;
    let mut res: u64 = 0;
    let t380: u64;
    if ((x0) >= (64u32)) {
        t380 = !(0u64);
    } else {
        t380 = ((1u64).wrapping_shl((((x0) as u64)) as u32)).wrapping_sub(1u64);
    }
    res = t380;
    return res;
}

#[inline(always)]
unsafe fn b20_r0sls_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], arg0: *const u8, x8: u32, x16: [uint32x4_t; 2], x24: u32) -> [uint64x2_t; 4] {
    let gid0 = l.group[0] * l.block[0] + lid0;
    let mut x32: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let mut live = k;
    let mut res: [uint64x2_t; 4] = [vdupq_n_u64(0); 4];
    let t381 = [vdupq_n_u64(((x8) as u64)); 4];
    let t382: [uint64x2_t; 4] = [vmovl_u32(vget_low_u32(x16[0])), vmovl_u32(vget_high_u32(x16[0])), vmovl_u32(vget_low_u32(x16[1])), vmovl_u32(vget_high_u32(x16[1]))];
    let t383: [uint64x2_t; 4] = [vaddq_u64(t381[0], t382[0]), vaddq_u64(t381[1], t382[1]), vaddq_u64(t381[2], t382[2]), vaddq_u64(t381[3], t382[3])];
    let t384 = b18_r0ls_p(l, lid0, live, arg0, t383, x24);
    x32 = t384;
    let t385: [uint64x2_t; 4];
    if ((x24) >= (8u32)) {
        t385 = x32;
    } else {
        let t386 = b19_s_p(l, lid0, live, (8u32).wrapping_mul(x24));
        let t387 = [vdupq_n_u64(t386); 4];
        let t388: [uint64x2_t; 4] = [vandq_u64(x32[0], t387[0]), vandq_u64(x32[1], t387[1]), vandq_u64(x32[2], t387[2]), vandq_u64(x32[3], t387[3])];
        t385 = t388;
    }
    res = t385;
    return res;
}

/// The group `l.group`'s threads, 8 at a time: `argv[j]` the address of argument `j`'s bytes (a buffer's address,
/// a value's bytes).
///
/// # Safety
///
/// `argv` holds the kernel's arguments, its buffers as large as it reads and writes; `l` is a 1-D group's launch.
pub unsafe fn parent_jump_w8(argv: *const *const u8, l: &Launch) {
    let n = l.block[0] * l.block[1] * l.block[2];
    let arg0: *const u8 = (*argv.add(0) as *const *const u8).read_unaligned();
    let arg1: *const u8 = (*argv.add(1) as *const *const u8).read_unaligned();
    let arg2: *const u8 = (*argv.add(2) as *const *const u8).read_unaligned();
    let arg3: *const u8 = (*argv.add(3) as *const *const u8).read_unaligned();
    let arg4: *const u8 = (*argv.add(4) as *const *const u8).read_unaligned();
    let arg5: *const u8 = (*argv.add(5) as *const *const u8).read_unaligned();
    let x48: *const u8 = *argv.add(6);
    let arg7: *const u8 = (*argv.add(7) as *const *const u8).read_unaligned();
    let x72: u32 = (*argv.add(8) as *const u32).read_unaligned();
    let mut t389: bool = !(arg7).is_null();
    if t389 {
        let mut t390: bool = ((l.group[0]) >= ((arg7.wrapping_add((((x72) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t390 {
            t390 = ((l.group[1]) >= ((arg7.wrapping_add(((((x72).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t391: bool = t390;
        if !t391 {
            t391 = ((l.group[2]) >= ((arg7.wrapping_add(((((x72).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t389 = t391;
    }
    if t389 {
        return;
    }
    let mut lid0 = 0u32;
    while lid0 + 8 <= n {
        let mut live = [vdupq_n_u32(u32::MAX); 2];
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1r2r3r4r5sg_f(l, lid0, arg0, arg1, arg2, arg3, arg4, arg5, x48);
        }
        lid0 += 8;
    }
    if lid0 < n {
        // Lanes part: the group's last threads, those under `n`.
        let t392: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t392;
        let gid0 = l.group[0] * l.block[0] + lid0;
        'chunk: {
            b0_r0r1r2r3r4r5sg_p(l, lid0, live, arg0, arg1, arg2, arg3, arg4, arg5, x48);
        }
    }
}
