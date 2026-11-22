//! `nearest_jump` at 8 lanes (2 NEON registers), written by rerail-ukl-vec's Rust (`rs.rs`) from its vector tape: do
//! not edit; `UKL_RS_WRITE=1 cargo test -p rerail-ukl-vec the_nearest_kernels_rust` writes it again.

#![allow(unused, unused_mut, unused_parens, unused_unsafe, non_snake_case, clippy::all, unsafe_op_in_unsafe_fn)]

use crate::Launch;
use core::arch::aarch64::*;
use core::sync::atomic::{AtomicU32, Ordering::Relaxed};

/// Lane `j`'s index in its register's `j`: `[0, 1, 2, 3]`, then `[4, 5, 6, 7]`, ...: a chunk's lanes.
const LANE: [[u32; 4]; 2] = [[0, 1, 2, 3], [4, 5, 6, 7]];

#[inline(always)]
unsafe fn b0_sssi(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: *const u8, x8: *const u8, x16: *const u8, x32: u32) {
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
    if vmaxvq_u32(vorrq_u32(t4[0], t4[1])) != 0 {
        live = [vbicq_u32(live[0], t4[0]), vbicq_u32(live[1], t4[1])];
    }
    if vmaxvq_u32(vorrq_u32(live[0], live[1])) != 0 {
        let t5 = b1_ai4(l, lid0, live, x0, x32);
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
        let mut t12 = [vdupq_n_u32(0); 2];
        loop {
            let t13 = [vdupq_n_u32(2147483648u32); 2];
            let t14: [uint32x4_t; 2] = [vandq_u32(x40[0], t13[0]), vandq_u32(x40[1], t13[1])];
            let t15: [uint32x4_t; 2] = [vceqq_u32(t14[0], t10[0]), vceqq_u32(t14[1], t10[1])];
            let t16: [uint32x4_t; 2] = [vcltq_u32(x56[0], t6[0]), vcltq_u32(x56[1], t6[1])];
            let t17: [uint32x4_t; 2] = [vandq_u32(t15[0], t16[0]), vandq_u32(t15[1], t16[1])];
            t11 = [vandq_u32(vbicq_u32(t11[0], t12[0]), t17[0]), vandq_u32(vbicq_u32(t11[1], t12[1]), t17[1])];
            if !(vmaxvq_u32(vorrq_u32(t11[0], t11[1])) != 0) { break; }
            let mut t18 = [vdupq_n_u32(0); 2];
            let t19: [uint32x4_t; 2] = [vaddq_u32(x48[0], x40[0]), vaddq_u32(x48[1], x40[1])];
            let t20 = b1_al4(l, lid0, t11, x0, t19);
            x64 = [vbslq_u32(t11[0], t20[0], x64[0]), vbslq_u32(t11[1], t20[1], x64[1])];
            x40 = [vbslq_u32(t11[0], x64[0], x40[0]), vbslq_u32(t11[1], x64[1], x40[1])];
            let t21 = [vdupq_n_u32(1u32); 2];
            let t22: [uint32x4_t; 2] = [vaddq_u32(x56[0], t21[0]), vaddq_u32(x56[1], t21[1])];
            let t23 = x56;
            x56 = [vbslq_u32(t11[0], t22[0], x56[0]), vbslq_u32(t11[1], t22[1], x56[1])];
            b2_ai4l(l, lid0, t11, x0, x32, x40);
        }
        let t24: [uint32x4_t; 2] = [vcgtq_u32(x56[0], t10[0]), vcgtq_u32(x56[1], t10[1])];
        let t25: [uint32x4_t; 2] = [vandq_u32(live[0], t24[0]), vandq_u32(live[1], t24[1])];
        if vmaxvq_u32(vorrq_u32(t25[0], t25[1])) != 0 {
            let t26 = b3_sl(l, lid0, t25, x8.wrapping_add((0u64).wrapping_mul(4u64) as usize), x56);
        }
    }
}

#[inline(always)]
unsafe fn b1_al4(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0b: *const u8, x0i: [uint32x4_t; 2]) -> [uint32x4_t; 2] {
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let t27 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], x0i[0]), vandq_u32(live[1], x0i[1]))));
    let t28: [uint32x4_t; 2] = [vbslq_u32(live[0], x0i[0], t27), vbslq_u32(live[1], x0i[1], t27)];
    let t29 = x0b as *const u32;
    let mut t30: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    t30[0] = vld1q_lane_u32::<0>(t29.add(vgetq_lane_u32::<0>(t28[0]) as usize), t30[0]);
    t30[0] = vld1q_lane_u32::<1>(t29.add(vgetq_lane_u32::<1>(t28[0]) as usize), t30[0]);
    t30[0] = vld1q_lane_u32::<2>(t29.add(vgetq_lane_u32::<2>(t28[0]) as usize), t30[0]);
    t30[0] = vld1q_lane_u32::<3>(t29.add(vgetq_lane_u32::<3>(t28[0]) as usize), t30[0]);
    t30[1] = vld1q_lane_u32::<0>(t29.add(vgetq_lane_u32::<0>(t28[1]) as usize), t30[1]);
    t30[1] = vld1q_lane_u32::<1>(t29.add(vgetq_lane_u32::<1>(t28[1]) as usize), t30[1]);
    t30[1] = vld1q_lane_u32::<2>(t29.add(vgetq_lane_u32::<2>(t28[1]) as usize), t30[1]);
    t30[1] = vld1q_lane_u32::<3>(t29.add(vgetq_lane_u32::<3>(t28[1]) as usize), t30[1]);
    res = [vbslq_u32(live[0], t30[0], res[0]), vbslq_u32(live[1], t30[1], res[1])];
    return res;
}

#[inline(always)]
unsafe fn b1_ai4(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0b: *const u8, x0i: u32) -> [uint32x4_t; 2] {
    let mut live = k;
    let mut res: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
    let mut t31: [uint32x4_t; 2];
    if vminvq_u32(vandq_u32(live[0], live[1])) != 0 {
        let t32 = (x0b as *const u32).add((x0i) as usize);
        t31 = [vld1q_u32(t32.add(0)), vld1q_u32(t32.add(4))];
    } else {
        let t33: [uint32x4_t; 2] = [vaddq_u32(vdupq_n_u32(x0i), vld1q_u32(LANE[0].as_ptr())), vaddq_u32(vdupq_n_u32(x0i), vld1q_u32(LANE[1].as_ptr()))];
        let t34 = vdupq_n_u32(vmaxvq_u32(vmaxq_u32(vandq_u32(live[0], t33[0]), vandq_u32(live[1], t33[1]))));
        let t35: [uint32x4_t; 2] = [vbslq_u32(live[0], t33[0], t34), vbslq_u32(live[1], t33[1], t34)];
        let t36 = x0b as *const u32;
        let mut t37: [uint32x4_t; 2] = [vdupq_n_u32(0); 2];
        t37[0] = vld1q_lane_u32::<0>(t36.add(vgetq_lane_u32::<0>(t35[0]) as usize), t37[0]);
        t37[0] = vld1q_lane_u32::<1>(t36.add(vgetq_lane_u32::<1>(t35[0]) as usize), t37[0]);
        t37[0] = vld1q_lane_u32::<2>(t36.add(vgetq_lane_u32::<2>(t35[0]) as usize), t37[0]);
        t37[0] = vld1q_lane_u32::<3>(t36.add(vgetq_lane_u32::<3>(t35[0]) as usize), t37[0]);
        t37[1] = vld1q_lane_u32::<0>(t36.add(vgetq_lane_u32::<0>(t35[1]) as usize), t37[1]);
        t37[1] = vld1q_lane_u32::<1>(t36.add(vgetq_lane_u32::<1>(t35[1]) as usize), t37[1]);
        t37[1] = vld1q_lane_u32::<2>(t36.add(vgetq_lane_u32::<2>(t35[1]) as usize), t37[1]);
        t37[1] = vld1q_lane_u32::<3>(t36.add(vgetq_lane_u32::<3>(t35[1]) as usize), t37[1]);
        t31 = t37;
    }
    res = [vbslq_u32(live[0], t31[0], res[0]), vbslq_u32(live[1], t31[1], res[1])];
    return res;
}

#[inline(always)]
unsafe fn b2_ai4l(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0b: *const u8, x0i: u32, x8: [uint32x4_t; 2]) {
    let mut live = k;
    let t38 = (x0b as *mut u32).add((x0i) as usize);
    if vminvq_u32(vandq_u32(live[0], live[1])) != 0 {
        vst1q_u32(t38.add(0), x8[0]);
        vst1q_u32(t38.add(4), x8[1]);
    } else {
        let mut t39w = 0u32;
        let t39 = core::hint::black_box(&raw mut t39w);
        core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[0]) != 0, t38.add(0), t39).write(vgetq_lane_u32::<0>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[0]) != 0, t38.add(1), t39).write(vgetq_lane_u32::<1>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[0]) != 0, t38.add(2), t39).write(vgetq_lane_u32::<2>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[0]) != 0, t38.add(3), t39).write(vgetq_lane_u32::<3>(x8[0]));
        core::hint::select_unpredictable(vgetq_lane_u32::<0>(live[1]) != 0, t38.add(4), t39).write(vgetq_lane_u32::<0>(x8[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<1>(live[1]) != 0, t38.add(5), t39).write(vgetq_lane_u32::<1>(x8[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<2>(live[1]) != 0, t38.add(6), t39).write(vgetq_lane_u32::<2>(x8[1]));
        core::hint::select_unpredictable(vgetq_lane_u32::<3>(live[1]) != 0, t38.add(7), t39).write(vgetq_lane_u32::<3>(x8[1]));
    }
}

#[inline(always)]
unsafe fn b3_sl(l: &Launch, lid0: u32, k: [uint32x4_t; 2], x0: *const u8, x8: [uint32x4_t; 2]) -> u32 {
    let mut live = k;
    let mut res: u32 = 0;
    let t40: [uint32x4_t; 2] = [vbslq_u32(live[0], x8[0], vdupq_n_u32(0)), vbslq_u32(live[1], x8[1], vdupq_n_u32(0))];
    if vmaxvq_u32(vorrq_u32(live[0], live[1])) != 0 { AtomicU32::from_ptr(x0 as *mut u32).fetch_max(vmaxvq_u32(vmaxq_u32(t40[0], t40[1])), Relaxed); }
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
    let mut t41: bool = !(x32).is_null();
    if t41 {
        let mut t42: bool = ((l.group[0]) >= ((x32.wrapping_add((((x40) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        if !t42 {
            t42 = ((l.group[1]) >= ((x32.wrapping_add(((((x40).wrapping_add(1u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        let mut t43: bool = t42;
        if !t43 {
            t43 = ((l.group[2]) >= ((x32.wrapping_add(((((x40).wrapping_add(2u32)) as u64)).wrapping_mul(4u64) as usize) as *const u32).read_unaligned()));
        }
        t41 = t43;
    }
    if t41 {
        return;
    }
    let mut lid0 = 0u32;
    while lid0 < n {
        let t44: [uint32x4_t; 2] = [vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[0].as_ptr())), vdupq_n_u32(n)), vcltq_u32(vaddq_u32(vdupq_n_u32(lid0), vld1q_u32(LANE[1].as_ptr())), vdupq_n_u32(n))];
        let mut live = t44;
        b0_sssi(l, lid0, live, x0, x8, x16, (l.group[0] * l.block[0] + lid0));
        lid0 += 8;
    }
}
