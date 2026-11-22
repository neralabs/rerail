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

/// Format 16's parent read (`reads::f16::parent`, `clif_f16_parent`) of every node of a tape, 8 nodes at a time (two
/// registers of 4 lanes), the nodes past the last whole 8 one at a time. A lane walks back from its node: the node
/// before it is its parent (an inner node, its slot even), or a leaf whose slot steps back to its previous sibling or
/// (bit 1) to the parent. Each round:
///
/// - the lanes' slots: in the first round, the 8 nodes before a chunk's are consecutive, so their slots are one run
///   of `8 * w` bytes of their block's array: one 64-byte load, cut into each lane's `w` bytes (its high bytes
///   zero) by a table lookup made once for the tape's width; in a later one (and where the run leaves its block or
///   the tape), each lane's address in vector code (its block's column, its row) and a load of each;
/// - an escaped slot (all ones: its side list's), the host's read of that lane, the rare case;
/// - the decisions in vector code: which lanes found their parent, stepped past the start, or stepped onto the parent.
///
/// # Safety
///
/// `v` is the view of the format 16 tape `t` (its offsets from `t`'s first byte), and `out` holds its nodes.
pub unsafe fn parent_all(t: &[u8], v: &rerail_clif::view::TapeView, out: &mut [u32]) {
    use rerail_tape_format::reads::f16::{parent, slot};
    const PART: u32 = 1024;
    const NONE: u32 = u32::MAX;
    unsafe {
        let n = v.nodes;
        let w = v.widths & 15;
        let ones = if w >= 8 { u64::MAX } else { (1u64 << (8 * w)) - 1 };
        let base = t.as_ptr();
        // Lane `k`'s `w` bytes of a run, little-endian, in its 8 (255: a zero byte).
        let mut ix = [255u8; 64];
        for k in 0..8 {
            for b in 0..w.min(8) {
                ix[8 * k as usize + b as usize] = (k * w + b) as u8;
            }
        }
        let ix = [vld1q_u8(ix.as_ptr()), vld1q_u8(ix.as_ptr().add(16)), vld1q_u8(ix.as_ptr().add(32)), vld1q_u8(ix.as_ptr().add(48))];
        const LANE: [u32; 8] = [0, 1, 2, 3, 4, 5, 6, 7];
        let (l0, l1) = (vld1q_u32(LANE.as_ptr()), vld1q_u32(LANE.as_ptr().add(4)));
        let (one, nodes) = (vdupq_n_u32(1), vdupq_n_u32(n));
        let (part, stride, c0, c1, wv) = (vdupq_n_u32(PART), vdupq_n_u32(v.stride), vdupq_n_u32(v.columns[0]), vdupq_n_u32(v.columns[1]), vdupq_n_u32(w));
        // A slot's low two bits and its step (`s >> 2`, as the read takes it), of each of 8 lanes' values.
        let split = |s: &[u64; 8]| {
            let mut f = [0u32; 8];
            let mut st = [0u32; 8];
            for k in 0..8 {
                f[k] = s[k] as u32 & 3;
                st[k] = (s[k] >> 2) as u32;
            }
            ([vld1q_u32(f.as_ptr()), vld1q_u32(f.as_ptr().add(4))], [vld1q_u32(st.as_ptr()), vld1q_u32(st.as_ptr().add(4))])
        };
        let mut c = 0u32;
        while c + 8 <= n {
            let mut j = [vaddq_u32(vdupq_n_u32(c), l0), vaddq_u32(vdupq_n_u32(c), l1)];
            let mut res = [vdupq_n_u32(NONE); 2];
            // The lanes still walking: not at the start.
            let mut on = [vtstq_u32(j[0], j[0]), vtstq_u32(j[1], j[1])];
            let mut first = true;
            loop {
                if vmaxvq_u32(vorrq_u32(on[0], on[1])) == 0 {
                    break;
                }
                // The node before each lane's (0 for a lane that is done: a node of the tape).
                let before = [vandq_u32(vsubq_u32(j[0], one), on[0]), vandq_u32(vsubq_u32(j[1], one), on[1])];
                let mut s = [0u64; 8];
                // The run's start: node c - 1's slot, the 8 in one block (and 64 bytes on the tape).
                let p = c.wrapping_sub(1);
                let at = (p / PART).wrapping_mul(v.stride).wrapping_add(v.columns[0]).wrapping_add(n.wrapping_sub(p / PART * PART).min(PART).wrapping_mul(v.columns[1])).wrapping_add(p % PART * w) as usize;
                if first && c != 0 && p % PART + 8 <= PART && w <= 8 && at + 64 <= t.len() {
                    // One run: the 8 slots of nodes c - 1 .. c + 6.
                    let p = base.add(at);
                    let tb = uint8x16x4_t(vld1q_u8(p), vld1q_u8(p.add(16)), vld1q_u8(p.add(32)), vld1q_u8(p.add(48)));
                    for (r, ix) in ix.iter().enumerate() {
                        let x = vreinterpretq_u64_u8(vqtbl4q_u8(tb, *ix));
                        s[2 * r] = vgetq_lane_u64::<0>(x);
                        s[2 * r + 1] = vgetq_lane_u64::<1>(x);
                    }
                } else {
                    // Each lane's: its block's slot column, its row.
                    let mut a = [0u32; 8];
                    for r in 0..2 {
                        let b = vshrq_n_u32::<10>(before[r]);
                        let rows = vminq_u32(vsubq_u32(nodes, vshlq_n_u32::<10>(b)), part);
                        let col = vmlaq_u32(vmlaq_u32(c0, b, stride), rows, c1);
                        vst1q_u32(a.as_mut_ptr().add(4 * r), vmlaq_u32(col, vandq_u32(before[r], vdupq_n_u32(PART - 1)), wv));
                    }
                    for k in 0..8 {
                        // Past the tape's end: all ones, the host's read of that lane below.
                        s[k] = if a[k] as usize + 8 <= t.len() { (base.add(a[k] as usize) as *const u64).read_unaligned() & ones } else { ones };
                    }
                }
                first = false;
                // An escaped slot: the side list's, by the host's read.
                for k in 0..8 {
                    if s[k] == ones {
                        let b = [vgetq_lane_u32::<0>(before[k / 4]), vgetq_lane_u32::<1>(before[k / 4]), vgetq_lane_u32::<2>(before[k / 4]), vgetq_lane_u32::<3>(before[k / 4])][k % 4];
                        s[k] = slot(t, v, b);
                    }
                }
                let (f, st) = split(&s);
                for r in 0..2 {
                    // An inner node before: the parent.
                    let found = vandq_u32(on[r], vceqzq_u32(vandq_u32(f[r], one)));
                    res[r] = vbslq_u32(found, before[r], res[r]);
                    let mut go = vbicq_u32(on[r], found);
                    // A step back past the start: no parent (a slot no tape's).
                    let past = vandq_u32(go, vcgeq_u32(vsubq_u32(st[r], one), j[r]));
                    go = vbicq_u32(go, past);
                    j[r] = vbslq_u32(go, vsubq_u32(j[r], st[r]), j[r]);
                    // A step onto the parent.
                    let up = vandq_u32(go, vtstq_u32(f[r], vdupq_n_u32(2)));
                    res[r] = vbslq_u32(up, j[r], res[r]);
                    go = vbicq_u32(go, up);
                    // A leaf's previous sibling: on from it, unless at the start.
                    on[r] = vandq_u32(go, vtstq_u32(j[r], j[r]));
                }
            }
            vst1q_u32(out.as_mut_ptr().add(c as usize), res[0]);
            vst1q_u32(out.as_mut_ptr().add(c as usize + 4), res[1]);
            c += 8;
        }
        for i in c..n {
            out[i as usize] = parent(t, v, i);
        }
    }
}
