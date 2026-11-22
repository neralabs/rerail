//! `nearest_jump` at 8 lanes (2 NEON registers), written by rerail-ukl-vec's Rust (`rs.rs`) from its vector tape: do
//! not edit; `UKL_RS_WRITE=1 cargo test -p rerail-ukl-vec the_nearest_kernels_rust` writes it again.

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
unsafe fn b0_sssi_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: *const u8, x8: *const u8, x16: *const u8, x32: u32) {
    let a0: u32 = (x16.wrapping_add(4u64 as usize) as *const u32).read_unaligned();
    let a1: u32 = (x16 as *const u32).read_unaligned();
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x56: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x64: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = k;
    let t1: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x32), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x32), vld1q_u32(LANE[1].as_ptr()))];
    let t2 = [vdupq_n_u32(a0); 2];
    let t3: [uint32x4_t; 2] = [vcgeq_u32(t1[0], t2[0]), vcgeq_u32(t1[1], t2[1])];
    let t4: [uint32x4_t; 2] = [vandq_u32(live[0], t3[0]), vandq_u32(live[1], t3[1])];
    if any(&t4) {
        live = [vbicq_u32(live[0], t4[0]), vbicq_u32(live[1], t4[1])];
    }
    if any(&live) {
        let t5 = b1_ai4_p(l, lid0, live, x0, x32);
        x40 = t5;
        let t6 = [vdupq_n_u32(a1); 2];
        let mut t7x = [0u32; 8];
        let mut t7y = [0u32; 8];
        for r in 0..2 { vst1q_u32(t7x.as_mut_ptr().add(4 * r), t1[r]); vst1q_u32(t7y.as_mut_ptr().add(4 * r), t6[r]); }
        for j in 0..8 { t7x[j] = (t7x[j] as u32).checked_div(t7y[j] as u32).unwrap_or(0) as u32; }
        let t8: [uint32x4_t; 2] = [vld1q_u32(t7x.as_ptr().add(0)), vld1q_u32(t7x.as_ptr().add(4))];
        let t9: [uint32x4_t; 2] = [vmulq_u32(t8[0], t6[0]), vmulq_u32(t8[1], t6[1])];
        x48 = t9;
        let t10 = [vdupq_n_u32(0u32); 2];
        x56 = t10;
        let mut t11 = live;
        loop {
            // Lanes part: each its own rounds.
            let t13 = [vdupq_n_u32(2147483648u32); 2];
            let t14: [uint32x4_t; 2] = [vandq_u32(x40[0], t13[0]), vandq_u32(x40[1], t13[1])];
            let t15: [uint32x4_t; 2] = [vceqq_u32(t14[0], t10[0]), vceqq_u32(t14[1], t10[1])];
            let t16: [uint32x4_t; 2] = [vcltq_u32(x56[0], t6[0]), vcltq_u32(x56[1], t6[1])];
            let t17: [uint32x4_t; 2] = [vandq_u32(t15[0], t16[0]), vandq_u32(t15[1], t16[1])];
            t11 = [vandq_u32(t11[0], t17[0]), vandq_u32(t11[1], t17[1])];
            if !(any(&t11)) { break; }
            let t19: [uint32x4_t; 2] = [vaddq_u32(x48[0], x40[0]), vaddq_u32(x48[1], x40[1])];
            let t20 = b1_al4_p(l, lid0, t11, x0, t19);
            x64 = t20;
            x40 = x64;
            let t21 = x56;
            x56 = [vsubq_u32(x56[0], t11[0]), vsubq_u32(x56[1], t11[1])];
            b2_ai4l_p(l, lid0, t11, x0, x32, x40);
        }
        let t22 = b3_sl_p(l, lid0, live, x8.wrapping_add((0u64).wrapping_mul(4u64) as usize), x56);
    }
}

#[inline(always)]
unsafe fn b0_sssi_f(l: &Launch, lid0: u32, x0: *const u8, x8: *const u8, x16: *const u8, x32: u32) {
    let a0: u32 = (x16.wrapping_add(4u64 as usize) as *const u32).read_unaligned();
    let a1: u32 = (x16 as *const u32).read_unaligned();
    let mut x40: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x48: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x56: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut x64: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    if ((x32) as u64 + 7) < ((a0) as u64) {
        let t23 = b1_ai4_f(l, lid0, x0, x32);
        x40 = t23;
        let t24: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x32), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x32), vld1q_u32(LANE[1].as_ptr()))];
        let t25 = [vdupq_n_u32(a1); 2];
        let mut t26x = [0u32; 8];
        let mut t26y = [0u32; 8];
        for r in 0..2 { vst1q_u32(t26x.as_mut_ptr().add(4 * r), t24[r]); vst1q_u32(t26y.as_mut_ptr().add(4 * r), t25[r]); }
        for j in 0..8 { t26x[j] = (t26x[j] as u32).checked_div(t26y[j] as u32).unwrap_or(0) as u32; }
        let t27: [uint32x4_t; 2] = [vld1q_u32(t26x.as_ptr().add(0)), vld1q_u32(t26x.as_ptr().add(4))];
        let t28: [uint32x4_t; 2] = [vmulq_u32(t27[0], t25[0]), vmulq_u32(t27[1], t25[1])];
        x48 = t28;
        let t29 = [vdupq_n_u32(0u32); 2];
        x56 = t29;
        let mut t30 = live;
        loop {
            // Lanes part: each its own rounds.
            let t32 = [vdupq_n_u32(2147483648u32); 2];
            let t33: [uint32x4_t; 2] = [vandq_u32(x40[0], t32[0]), vandq_u32(x40[1], t32[1])];
            let t34: [uint32x4_t; 2] = [vceqq_u32(t33[0], t29[0]), vceqq_u32(t33[1], t29[1])];
            let t35: [uint32x4_t; 2] = [vcltq_u32(x56[0], t25[0]), vcltq_u32(x56[1], t25[1])];
            let t36: [uint32x4_t; 2] = [vandq_u32(t34[0], t35[0]), vandq_u32(t34[1], t35[1])];
            t30 = [vandq_u32(t30[0], t36[0]), vandq_u32(t30[1], t36[1])];
            if !(any(&t30)) { break; }
            let t38: [uint32x4_t; 2] = [vaddq_u32(x48[0], x40[0]), vaddq_u32(x48[1], x40[1])];
            let t39 = b1_al4_p(l, lid0, t30, x0, t38);
            x64 = t39;
            x40 = x64;
            let t40 = x56;
            x56 = [vsubq_u32(x56[0], t30[0]), vsubq_u32(x56[1], t30[1])];
            b2_ai4l_p(l, lid0, t30, x0, x32, x40);
        }
        let t41 = b3_sl_f(l, lid0, x8.wrapping_add((0u64).wrapping_mul(4u64) as usize), x56);
    } else if (((x32) as u64 + 7) <= u32::MAX as u64 && ((x32) as u64) >= ((a0) as u64)) {
        return;
    } else {
        // Lanes part: the chunk's lanes on both sides of the bound.
        let t42: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x32), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x32), vld1q_u32(LANE[1].as_ptr()))];
        let t43 = [vdupq_n_u32(a0); 2];
        let t44: [uint32x4_t; 2] = [vcgeq_u32(t42[0], t43[0]), vcgeq_u32(t42[1], t43[1])];
        let t45: [uint32x4_t; 2] = [vandq_u32(live[0], t44[0]), vandq_u32(live[1], t44[1])];
        if any(&t45) {
            live = [vbicq_u32(live[0], t45[0]), vbicq_u32(live[1], t45[1])];
        }
        if any(&live) {
            let t46 = b1_ai4_p(l, lid0, live, x0, x32);
            x40 = t46;
            let t47 = [vdupq_n_u32(a1); 2];
            let mut t48x = [0u32; 8];
            let mut t48y = [0u32; 8];
            for r in 0..2 { vst1q_u32(t48x.as_mut_ptr().add(4 * r), t42[r]); vst1q_u32(t48y.as_mut_ptr().add(4 * r), t47[r]); }
            for j in 0..8 { t48x[j] = (t48x[j] as u32).checked_div(t48y[j] as u32).unwrap_or(0) as u32; }
            let t49: [uint32x4_t; 2] = [vld1q_u32(t48x.as_ptr().add(0)), vld1q_u32(t48x.as_ptr().add(4))];
            let t50: [uint32x4_t; 2] = [vmulq_u32(t49[0], t47[0]), vmulq_u32(t49[1], t47[1])];
            x48 = t50;
            let t51 = [vdupq_n_u32(0u32); 2];
            x56 = t51;
            let mut t52 = live;
            loop {
                // Lanes part: each its own rounds.
                let t54 = [vdupq_n_u32(2147483648u32); 2];
                let t55: [uint32x4_t; 2] = [vandq_u32(x40[0], t54[0]), vandq_u32(x40[1], t54[1])];
                let t56: [uint32x4_t; 2] = [vceqq_u32(t55[0], t51[0]), vceqq_u32(t55[1], t51[1])];
                let t57: [uint32x4_t; 2] = [vcltq_u32(x56[0], t47[0]), vcltq_u32(x56[1], t47[1])];
                let t58: [uint32x4_t; 2] = [vandq_u32(t56[0], t57[0]), vandq_u32(t56[1], t57[1])];
                t52 = [vandq_u32(t52[0], t58[0]), vandq_u32(t52[1], t58[1])];
                if !(any(&t52)) { break; }
                let t60: [uint32x4_t; 2] = [vaddq_u32(x48[0], x40[0]), vaddq_u32(x48[1], x40[1])];
                let t61 = b1_al4_p(l, lid0, t52, x0, t60);
                x64 = t61;
                x40 = x64;
                let t62 = x56;
                x56 = [vsubq_u32(x56[0], t52[0]), vsubq_u32(x56[1], t52[1])];
                b2_ai4l_p(l, lid0, t52, x0, x32, x40);
            }
            let t63 = b3_sl_p(l, lid0, live, x8.wrapping_add((0u64).wrapping_mul(4u64) as usize), x56);
        }
    }
}

#[inline(always)]
unsafe fn b1_al4_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0b: *const u8, x0i: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t64 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x0i[0]), vandq_u32(live[1], x0i[1]))));
    let t65: [uint32x4_t; 2] = [vbslq_u32(live[0], x0i[0], t64), vbslq_u32(live[1], x0i[1], t64)];
    let t66 = x0b as *const u32;
    let mut t67: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    t67[0] = vld1q_lane_u32::<0>(t66.add(vgetq_lane_u32::<0>(t65[0]) as usize), t67[0]);
    t67[0] = vld1q_lane_u32::<1>(t66.add(vgetq_lane_u32::<1>(t65[0]) as usize), t67[0]);
    t67[0] = vld1q_lane_u32::<2>(t66.add(vgetq_lane_u32::<2>(t65[0]) as usize), t67[0]);
    t67[0] = vld1q_lane_u32::<3>(t66.add(vgetq_lane_u32::<3>(t65[0]) as usize), t67[0]);
    t67[1] = vld1q_lane_u32::<0>(t66.add(vgetq_lane_u32::<0>(t65[1]) as usize), t67[1]);
    t67[1] = vld1q_lane_u32::<1>(t66.add(vgetq_lane_u32::<1>(t65[1]) as usize), t67[1]);
    t67[1] = vld1q_lane_u32::<2>(t66.add(vgetq_lane_u32::<2>(t65[1]) as usize), t67[1]);
    t67[1] = vld1q_lane_u32::<3>(t66.add(vgetq_lane_u32::<3>(t65[1]) as usize), t67[1]);
    res = t67;
    return res;
}

#[inline(always)]
unsafe fn b1_ai4_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0b: *const u8, x0i: u32) -> [uint32x4_t; 2] {
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut t68: [uint32x4_t; 2];
    if all(&live) {
        let t69 = (x0b as *const u32).add((x0i) as usize);
        t68 = [vld1q_u32(t69.add(0)), vld1q_u32(t69.add(4))];
    } else {
        let t70: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x0i), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x0i), vld1q_u32(LANE[1].as_ptr()))];
        let t71 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t70[0]), vandq_u32(live[1], t70[1]))));
        let t72: [uint32x4_t; 2] = [vbslq_u32(live[0], t70[0], t71), vbslq_u32(live[1], t70[1], t71)];
        let t73 = x0b as *const u32;
        let mut t74: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t74[0] = vld1q_lane_u32::<0>(t73.add(vgetq_lane_u32::<0>(t72[0]) as usize), t74[0]);
        t74[0] = vld1q_lane_u32::<1>(t73.add(vgetq_lane_u32::<1>(t72[0]) as usize), t74[0]);
        t74[0] = vld1q_lane_u32::<2>(t73.add(vgetq_lane_u32::<2>(t72[0]) as usize), t74[0]);
        t74[0] = vld1q_lane_u32::<3>(t73.add(vgetq_lane_u32::<3>(t72[0]) as usize), t74[0]);
        t74[1] = vld1q_lane_u32::<0>(t73.add(vgetq_lane_u32::<0>(t72[1]) as usize), t74[1]);
        t74[1] = vld1q_lane_u32::<1>(t73.add(vgetq_lane_u32::<1>(t72[1]) as usize), t74[1]);
        t74[1] = vld1q_lane_u32::<2>(t73.add(vgetq_lane_u32::<2>(t72[1]) as usize), t74[1]);
        t74[1] = vld1q_lane_u32::<3>(t73.add(vgetq_lane_u32::<3>(t72[1]) as usize), t74[1]);
        t68 = t74;
    }
    res = t68;
    return res;
}

#[inline(always)]
unsafe fn b1_ai4_f(l: &Launch, lid0: u32, x0b: *const u8, x0i: u32) -> [uint32x4_t; 2] {
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t75 = (x0b as *const u32).add((x0i) as usize);
    let t76: [uint32x4_t; 2] = [vld1q_u32(t75.add(0)), vld1q_u32(t75.add(4))];
    res = t76;
    return res;
}

#[inline(always)]
unsafe fn b2_ai4l_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0b: *const u8, x0i: u32, x8: [uint32x4_t; 2]) {
    let mut live = k;
    let t77 = (x0b as *mut u32).add((x0i) as usize);
    if all(&live) {
        vst1q_u32(t77.add(0), x8[0]);
        vst1q_u32(t77.add(4), x8[1]);
    } else {
        let mut t78w = 0u32;
        let t78 = core::hint::black_box(&raw mut t78w);
        core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t77.add(0), t78).write(vgetq_lane_u32::<0>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t77.add(1), t78).write(vgetq_lane_u32::<1>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t77.add(2), t78).write(vgetq_lane_u32::<2>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t77.add(3), t78).write(vgetq_lane_u32::<3>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t77.add(4), t78).write(vgetq_lane_u32::<0>(x8[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t77.add(5), t78).write(vgetq_lane_u32::<1>(x8[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t77.add(6), t78).write(vgetq_lane_u32::<2>(x8[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t77.add(7), t78).write(vgetq_lane_u32::<3>(x8[1]));
    }
}

#[inline(always)]
unsafe fn b3_sl_p(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: *const u8, x8: [uint32x4_t; 2]) -> u32 {
    let mut live = k;
    let mut res: u32 = 0;
    let t79: [uint32x4_t; 2] = [vbslq_u32(live[0], x8[0], vdupq_n_u32(0)), vbslq_u32(live[1], x8[1], vdupq_n_u32(0))];
    let t80 = vmaxvq_u32(vmaxq_u32(t79[0], t79[1]));
    if t80 != 0 { AtomicU32::from_ptr(x0 as *mut u32).fetch_max(t80, Relaxed); }
    return res;
}

#[inline(always)]
unsafe fn b3_sl_f(l: &Launch, lid0: u32, x0: *const u8, x8: [uint32x4_t; 2]) -> u32 {
    let mut live = [vdupq_n_u32(u32::MAX); 2];
    let mut res: u32 = 0;
    let t81 = vmaxvq_u32(vmaxq_u32(x8[0], x8[1]));
    if t81 != 0 { AtomicU32::from_ptr(x0 as *mut u32).fetch_max(t81, Relaxed); }
    return res;
}

/// The group `l.group`'s threads, 8 at a time: `argv[j]` the address of argument `j`'s bytes (a buffer's address,
/// a value's bytes).
///
/// # Safety
///
/// `argv` holds the kernel's arguments, its buffers as large as it reads and writes; `l` is a 1-D group's launch.
pub unsafe fn nearest_jump_w8(argv: *const *const u8, l: &Launch) {
    let n = l.block[0] * l.block[1] * l.block[2];
    let x0: *const u8 = (*argv.add(0) as *const *const u8).read_unaligned();
    let x8: *const u8 = (*argv.add(1) as *const *const u8).read_unaligned();
    let x16: *const u8 = *argv.add(2);
    let x32: *const u8 = (*argv.add(3) as *const *const u8).read_unaligned();
    let x40: u32 = (*argv.add(4) as *const u32).read_unaligned();
    let mut t82: bool = !(x32).is_null();
    if t82 {
        let mut t83: bool = ((l.group[0]) >= ((x32.wrapping_add((((x40) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t83 {
            t83 = ((l.group[1]) >= ((x32.wrapping_add(((((x40).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t84: bool = t83;
        if !t84 {
            t84 = ((l.group[2]) >= ((x32.wrapping_add(((((x40).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t82 = t84;
    }
    if t82 {
        return;
    }
    let mut lid0 = 0u32;
    while lid0 + 8 <= n {
        let mut live = [vdupq_n_u32(u32::MAX); 2];
        'chunk: {
            b0_sssi_f(l, lid0, x0, x8, x16, (l.group[0] * l.block[0] + lid0));
        }
        lid0 += 8;
    }
    if lid0 < n {
        // Lanes part: the group's last threads, those under `n`.
        let t85: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t85;
        'chunk: {
            b0_sssi_p(l, lid0, live, x0, x8, x16, (l.group[0] * l.block[0] + lid0));
        }
    }
}
