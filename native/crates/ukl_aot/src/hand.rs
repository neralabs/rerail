//! The kernels vectorized by hand, with NEON intrinsics, as a host kernel would be written: the bar the Rust
//! rerail-ukl-vec writes is compared with.

use crate::FINAL;
use core::arch::aarch64::*;

/// Lane `j`'s `u32` at `b + i[j]` (`ldr w, [x, w, uxtw #2]` each).
#[inline(always)]
unsafe fn gather(b: *const u32, i: uint32x4_t) -> uint32x4_t {
    unsafe {
        let mut r = vdupq_n_u32(0);
        r = vld1q_lane_u32::<0>(b.add(vgetq_lane_u32::<0>(i) as usize), r);
        r = vld1q_lane_u32::<1>(b.add(vgetq_lane_u32::<1>(i) as usize), r);
        r = vld1q_lane_u32::<2>(b.add(vgetq_lane_u32::<2>(i) as usize), r);
        r = vld1q_lane_u32::<3>(b.add(vgetq_lane_u32::<3>(i) as usize), r);
        r
    }
}

/// `nearest_jump` 8 cells at a time (two registers of 4 lanes), the rest one at a time. Each round the lanes that
/// go on read their target's state; the others read their own cell's, which is their state (each cell is written
/// by its own lane only), so the round's value is the load's in every lane and its store is the two registers'.
/// Returns the most steps a cell took.
///
/// # Safety
///
/// `state` is `cells` cells, each a state of its set's block of `nodes` (`FINAL`, or a node under `nodes`).
pub unsafe fn nearest_jump(state: *mut u32, nodes: u32, cells: u32) -> u32 {
    unsafe {
        const LANE: [u32; 8] = [0, 1, 2, 3, 4, 5, 6, 7];
        let (l0, l1) = (vld1q_u32(LANE.as_ptr()), vld1q_u32(LANE.as_ptr().add(4)));
        let (fin, nv) = (vdupq_n_u32(FINAL), vdupq_n_u32(nodes));
        let mut most = vdupq_n_u32(0);
        let mut c0 = 0u32;
        while c0 + 8 <= cells {
            let p = state.add(c0 as usize);
            let (cell0, cell1) = (vaddq_u32(vdupq_n_u32(c0), l0), vaddq_u32(vdupq_n_u32(c0), l1));
            let (mut v0, mut v1) = (vld1q_u32(p), vld1q_u32(p.add(4)));
            // Each lane's set's first cell: once a cell, by the integer unit.
            let mut b = [0u32; 8];
            for (j, b) in b.iter_mut().enumerate() {
                *b = (c0 + j as u32) / nodes * nodes;
            }
            let (b0, b1) = (vld1q_u32(b.as_ptr()), vld1q_u32(b.as_ptr().add(4)));
            let (mut s0, mut s1) = (vdupq_n_u32(0), vdupq_n_u32(0));
            loop {
                let on0 = vandq_u32(vceqzq_u32(vandq_u32(v0, fin)), vcltq_u32(s0, nv));
                let on1 = vandq_u32(vceqzq_u32(vandq_u32(v1, fin)), vcltq_u32(s1, nv));
                if vmaxvq_u32(vorrq_u32(on0, on1)) == 0 {
                    break;
                }
                v0 = gather(state, vbslq_u32(on0, vaddq_u32(b0, v0), cell0));
                v1 = gather(state, vbslq_u32(on1, vaddq_u32(b1, v1), cell1));
                s0 = vsubq_u32(s0, on0);
                s1 = vsubq_u32(s1, on1);
                vst1q_u32(p, v0);
                vst1q_u32(p.add(4), v1);
            }
            most = vmaxq_u32(most, vmaxq_u32(s0, s1));
            c0 += 8;
        }
        let mut most = vmaxvq_u32(most);
        for cell in c0..cells {
            let mut v = *state.add(cell as usize);
            let base = cell / nodes * nodes;
            let mut steps = 0;
            while v & FINAL == 0 && steps < nodes {
                v = *state.add((base + v) as usize);
                steps += 1;
                *state.add(cell as usize) = v;
            }
            most = most.max(steps);
        }
        most
    }
}

/// Lane `j`'s `x[j]` written to `b + i[j]`.
#[inline(always)]
unsafe fn scatter(b: *mut u32, i: uint32x4_t, x: uint32x4_t) {
    unsafe {
        *b.add(vgetq_lane_u32::<0>(i) as usize) = vgetq_lane_u32::<0>(x);
        *b.add(vgetq_lane_u32::<1>(i) as usize) = vgetq_lane_u32::<1>(x);
        *b.add(vgetq_lane_u32::<2>(i) as usize) = vgetq_lane_u32::<2>(x);
        *b.add(vgetq_lane_u32::<3>(i) as usize) = vgetq_lane_u32::<3>(x);
    }
}

/// `nearest_jump` as `nearest_jump`, the threads in another order: in each group of `block` cells (a multiple of 8),
/// lane `j` takes the cells `j * block / 8 + k`, `k` in turn. A lane walks its own stretch in order, as the scalar
/// loop does, so a cell whose parent lies just below it finds the parent's state final (the same lane wrote it a
/// chunk before); consecutive lanes no longer wait on each other. The cells past the last whole group run one at a
/// time. Returns the most steps a cell took.
///
/// # Safety
///
/// As `nearest_jump`'s.
pub unsafe fn nearest_jump_strided(state: *mut u32, nodes: u32, cells: u32, block: u32) -> u32 {
    unsafe {
        let per = block / 8;
        const LANE: [u32; 8] = [0, 1, 2, 3, 4, 5, 6, 7];
        let pv = vdupq_n_u32(per);
        let (o0, o1) = (vmulq_u32(vld1q_u32(LANE.as_ptr()), pv), vmulq_u32(vld1q_u32(LANE.as_ptr().add(4)), pv));
        let (fin, nv) = (vdupq_n_u32(FINAL), vdupq_n_u32(nodes));
        let mut most = vdupq_n_u32(0);
        let whole = cells / block * block;
        let mut g = 0u32;
        while g < whole {
            for k in 0..per {
                let c = vdupq_n_u32(g + k);
                let (cell0, cell1) = (vaddq_u32(c, o0), vaddq_u32(c, o1));
                let (mut v0, mut v1) = (gather(state, cell0), gather(state, cell1));
                let mut b = [0u32; 8];
                for (j, b) in b.iter_mut().enumerate() {
                    *b = (g + k + j as u32 * per) / nodes * nodes;
                }
                let (b0, b1) = (vld1q_u32(b.as_ptr()), vld1q_u32(b.as_ptr().add(4)));
                let (mut s0, mut s1) = (vdupq_n_u32(0), vdupq_n_u32(0));
                loop {
                    let on0 = vandq_u32(vceqzq_u32(vandq_u32(v0, fin)), vcltq_u32(s0, nv));
                    let on1 = vandq_u32(vceqzq_u32(vandq_u32(v1, fin)), vcltq_u32(s1, nv));
                    if vmaxvq_u32(vorrq_u32(on0, on1)) == 0 {
                        break;
                    }
                    v0 = gather(state, vbslq_u32(on0, vaddq_u32(b0, v0), cell0));
                    v1 = gather(state, vbslq_u32(on1, vaddq_u32(b1, v1), cell1));
                    s0 = vsubq_u32(s0, on0);
                    s1 = vsubq_u32(s1, on1);
                    scatter(state, cell0, v0);
                    scatter(state, cell1, v1);
                }
                most = vmaxq_u32(most, vmaxq_u32(s0, s1));
            }
            g += block;
        }
        let mut most = vmaxvq_u32(most);
        for cell in whole..cells {
            let mut v = *state.add(cell as usize);
            let base = cell / nodes * nodes;
            let mut steps = 0;
            while v & FINAL == 0 && steps < nodes {
                v = *state.add((base + v) as usize);
                steps += 1;
                *state.add(cell as usize) = v;
            }
            most = most.max(steps);
        }
        most
    }
}
