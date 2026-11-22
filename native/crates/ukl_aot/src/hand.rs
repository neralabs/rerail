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

/// The slot reads of 8 lanes of a format 16 tape (`reads::f16::slot`): the constants of its slot array, made once.
struct Slots<'a> {
    t: &'a [u8],
    v: &'a rerail_clif::view::TapeView,
    w: u32,
    ones: u64,
    /// Lane `k`'s `w` bytes of a run of 8 slots, little-endian, in its 8 (255: a zero byte).
    ix: [uint8x16_t; 4],
    /// The node before an escaped slot read last, and its value (siblings' walks meet at one checkpoint).
    memo: (u32, u64),
}

const F16_PART: u32 = rerail_clif::view::F16_PART;

impl<'a> Slots<'a> {
    unsafe fn new(t: &'a [u8], v: &'a rerail_clif::view::TapeView) -> Slots<'a> {
        let w = v.widths & 15;
        let mut ix = [255u8; 64];
        for k in 0..8 {
            for b in 0..w.min(8) {
                ix[8 * k as usize + b as usize] = (k * w + b) as u8;
            }
        }
        let ix = unsafe { [vld1q_u8(ix.as_ptr()), vld1q_u8(ix.as_ptr().add(16)), vld1q_u8(ix.as_ptr().add(32)), vld1q_u8(ix.as_ptr().add(48))] };
        Slots { t, v, w, ones: if w >= 8 { u64::MAX } else { (1u64 << (8 * w)) - 1 }, ix, memo: (u32::MAX, 0) }
    }

    /// The slots of the nodes `at` of the lanes `on` marks (the others' are any). Where the lanes' nodes are the 8
    /// from `first` up, in one block, one run of `8 * w` bytes: one 64-byte load, cut into the lanes' `w` bytes by a
    /// table lookup; else each lane's address in vector code (its block's slot column, its row) and a load of each.
    /// An escaped slot (all ones) of a lane `on` marks, its side list's: the lanes' searches at once.
    #[inline(always)]
    unsafe fn read(&mut self, at: &[uint32x4_t; 2], on: &[uint32x4_t; 2], first: Option<u32>) -> [uint64x2_t; 4] {
        let (t, v, w) = (self.t, self.v, self.w);
        unsafe {
            let base = t.as_ptr();
            let ones = vdupq_n_u64(self.ones);
            let run = first.and_then(|p| {
                let from = (p / F16_PART).wrapping_mul(v.stride).wrapping_add(v.columns[0]).wrapping_add(v.nodes.wrapping_sub(p / F16_PART * F16_PART).min(F16_PART).wrapping_mul(v.columns[1])).wrapping_add(p % F16_PART * w) as usize;
                (p % F16_PART + 8 <= F16_PART && w <= 8 && from + 64 <= t.len()).then_some(from)
            });
            let mut s = if let Some(from) = run {
                let p = base.add(from);
                let tb = uint8x16x4_t(vld1q_u8(p), vld1q_u8(p.add(16)), vld1q_u8(p.add(32)), vld1q_u8(p.add(48)));
                let cut = |r: usize| vreinterpretq_u64_u8(vqtbl4q_u8(tb, self.ix[r]));
                [cut(0), cut(1), cut(2), cut(3)]
            } else {
                let (part, nodes) = (vdupq_n_u32(F16_PART), vdupq_n_u32(v.nodes));
                let addr = |x: uint32x4_t| {
                    let b = vshrq_n_u32::<10>(x);
                    let rows = vminq_u32(vsubq_u32(nodes, vshlq_n_u32::<10>(b)), part);
                    let col = vmlaq_u32(vmlaq_u32(vdupq_n_u32(v.columns[0]), b, vdupq_n_u32(v.stride)), rows, vdupq_n_u32(v.columns[1]));
                    vmlaq_u32(col, vandq_u32(x, vdupq_n_u32(F16_PART - 1)), vdupq_n_u32(w))
                };
                let a = [addr(at[0]), addr(at[1])];
                let word = |p: u32| (base.add(p as usize) as *const u64).read_unaligned();
                if vmaxvq_u32(vmaxq_u32(a[0], a[1])) as usize + 8 <= t.len() {
                    // Every lane's word on the tape: one check of the largest address.
                    let pair = |x: uint32x4_t, hi: bool| {
                        let (p, q) = if hi { (vgetq_lane_u32::<2>(x), vgetq_lane_u32::<3>(x)) } else { (vgetq_lane_u32::<0>(x), vgetq_lane_u32::<1>(x)) };
                        vandq_u64(vcombine_u64(vcreate_u64(word(p)), vcreate_u64(word(q))), ones)
                    };
                    [pair(a[0], false), pair(a[0], true), pair(a[1], false), pair(a[1], true)]
                } else {
                    // Past the tape's end (no tape's: the host's load would refuse it): taken as escaped.
                    let mut a8 = [0u32; 8];
                    vst1q_u32(a8.as_mut_ptr(), a[0]);
                    vst1q_u32(a8.as_mut_ptr().add(4), a[1]);
                    let load = |k: usize| if a8[k] as usize + 8 <= t.len() { word(a8[k]) } else { u64::MAX };
                    let pair = |r: usize| vandq_u64(vcombine_u64(vcreate_u64(load(2 * r)), vcreate_u64(load(2 * r + 1))), ones);
                    [pair(0), pair(1), pair(2), pair(3)]
                }
            };
            // Escaped slots of live lanes: one compare of the 8, the rare case past it.
            let e = [vceqq_u64(s[0], ones), vceqq_u64(s[1], ones), vceqq_u64(s[2], ones), vceqq_u64(s[3], ones)];
            let esc = [vandq_u32(on[0], vcombine_u32(vmovn_u64(e[0]), vmovn_u64(e[1]))), vandq_u32(on[1], vcombine_u32(vmovn_u64(e[2]), vmovn_u64(e[3])))];
            if vmaxvq_u32(vorrq_u32(esc[0], esc[1])) != 0 {
                let mut nodes = [0u32; 8];
                let mut need = [0u32; 8];
                let mut s8 = [0u64; 8];
                vst1q_u32(nodes.as_mut_ptr(), at[0]);
                vst1q_u32(nodes.as_mut_ptr().add(4), at[1]);
                vst1q_u32(need.as_mut_ptr(), esc[0]);
                vst1q_u32(need.as_mut_ptr().add(4), esc[1]);
                for r in 0..4 {
                    vst1q_u64(s8.as_mut_ptr().add(2 * r), s[r]);
                }
                let mut any = 0;
                for k in 0..8 {
                    if need[k] != 0 && nodes[k] == self.memo.0 {
                        s8[k] = self.memo.1;
                        need[k] = 0;
                    }
                    any |= need[k];
                }
                if any != 0 {
                    let side = side8(t, v, 1, &nodes, &need);
                    for k in 0..8 {
                        if need[k] != 0 {
                            s8[k] = side[k];
                            self.memo = (nodes[k], side[k]);
                        }
                    }
                }
                s = [0, 1, 2, 3].map(|r| vld1q_u64(s8.as_ptr().add(2 * r)));
            }
            s
        }
    }
}

/// A slot's low two bits and its step (`s >> 2`, as the read takes it), of each of 8 lanes' values: narrowing moves.
#[inline(always)]
unsafe fn split(s: &[uint64x2_t; 4]) -> ([uint32x4_t; 2], [uint32x4_t; 2]) {
    unsafe {
        let lo = |a: uint64x2_t, b: uint64x2_t| vcombine_u32(vmovn_u64(a), vmovn_u64(b));
        let st = |a: uint64x2_t, b: uint64x2_t| vcombine_u32(vmovn_u64(vshrq_n_u64::<2>(a)), vmovn_u64(vshrq_n_u64::<2>(b)));
        let three = vdupq_n_u32(3);
        ([vandq_u32(lo(s[0], s[1]), three), vandq_u32(lo(s[2], s[3]), three)], [st(s[0], s[1]), st(s[2], s[3])])
    }
}

const LANES8: [u32; 8] = [0, 1, 2, 3, 4, 5, 6, 7];

/// Format 16's parent read (`reads::f16::parent`, `clif_f16_parent`) of every node of a tape, the kernel's algorithm:
/// 8 nodes at a time (two registers of 4 lanes), each lane its node's walk, the nodes past the last whole 8 one at a
/// time. A lane walks back from its node: the node before it is its parent (an inner node, its slot even), or a leaf
/// whose slot steps back to its previous sibling or (bit 1) to the parent. Each round:
///
/// - the lanes' slots (`Slots::read`): in the first round the 8 nodes before a chunk's are consecutive, one run;
/// - the decisions in vector code: which lanes found their parent, stepped past the start, or stepped onto the parent.
///
/// # Safety
///
/// `v` is the view of the format 16 tape `t` (its offsets from `t`'s first byte), and `out` holds its nodes.
pub unsafe fn parent_all(t: &[u8], v: &rerail_clif::view::TapeView, out: &mut [u32]) {
    use rerail_tape_format::reads::f16::parent;
    const NONE: u32 = u32::MAX;
    unsafe {
        let n = v.nodes;
        let mut slots = Slots::new(t, v);
        let (l0, l1) = (vld1q_u32(LANES8.as_ptr()), vld1q_u32(LANES8.as_ptr().add(4)));
        let one = vdupq_n_u32(1);
        let mut c = 0u32;
        while c + 8 <= n {
            let mut j = [vaddq_u32(vdupq_n_u32(c), l0), vaddq_u32(vdupq_n_u32(c), l1)];
            let mut res = [vdupq_n_u32(NONE); 2];
            // The lanes still walking: not at the start.
            let mut on = [vtstq_u32(j[0], j[0]), vtstq_u32(j[1], j[1])];
            let mut first = c.checked_sub(1);
            loop {
                if vmaxvq_u32(vorrq_u32(on[0], on[1])) == 0 {
                    break;
                }
                // The node before each lane's (0 for a lane that is done: a node of the tape).
                let before = [vandq_u32(vsubq_u32(j[0], one), on[0]), vandq_u32(vsubq_u32(j[1], one), on[1])];
                let (f, st) = split(&slots.read(&before, &on, first.take()));
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

/// 8 lanes' `x[link]`: a table lookup of the lanes' bytes, `link` each lane's lane (0 to 7).
unsafe fn take8(x: &[uint32x4_t; 2], link: &[uint32x4_t; 2]) -> [uint32x4_t; 2] {
    unsafe {
        let tb = uint8x16x2_t(vreinterpretq_u8_u32(x[0]), vreinterpretq_u8_u32(x[1]));
        let bytes = vdupq_n_u32(0x0302_0100);
        let at = |l: uint32x4_t| vreinterpretq_u32_u8(vqtbl2q_u8(tb, vreinterpretq_u8_u32(vmlaq_u32(bytes, l, vdupq_n_u32(0x0404_0404)))));
        [at(link[0]), at(link[1])]
    }
}

/// Every node's parent, as `parent_all`'s, from one slot read a node and the answers before it: a walk's only state
/// is its `j`, so a walk that steps from node `i` to its previous sibling's `j' = i - step` goes on as node `j'`'s
/// own, and `i`'s parent is `j'`'s, an answer already made (`j' < i`). Per chunk of 8 nodes:
///
/// - the 8 slots of the nodes before (`Slots::read`, one run);
/// - each lane's first step in vector code: its parent found (the node before, or the parent stepped onto, or none),
///   or a sibling `j'`;
/// - a `j'` before the chunk, its answer in `out`; one in the chunk, another lane's (to its left), taken by pointer
///   jumping in registers: 3 rounds of a table lookup of the lanes' answers and links.
///
/// Not the kernel's algorithm (its threads are independent): what a read of every node can do in one pass.
///
/// # Safety
///
/// As `parent_all`'s.
pub unsafe fn parent_all_scan(t: &[u8], v: &rerail_clif::view::TapeView, out: &mut [u32]) {
    const NONE: u32 = u32::MAX;
    let n = v.nodes;
    assert!(out.len() >= n as usize, "parent_all_scan: an out shorter than the tape's nodes");
    unsafe {
        let mut slots = Slots::new(t, v);
        let (l0, l1) = (vld1q_u32(LANES8.as_ptr()), vld1q_u32(LANES8.as_ptr().add(4)));
        let one = vdupq_n_u32(1);
        let mut c = 0u32;
        while c < n {
            // The chunk's lanes: nodes c .. c + 8, those past the tape's end off.
            let i = [vaddq_u32(vdupq_n_u32(c), l0), vaddq_u32(vdupq_n_u32(c), l1)];
            let live = [vcltq_u32(i[0], vdupq_n_u32(n)), vcltq_u32(i[1], vdupq_n_u32(n))];
            // A lane walking: not at the start, in the tape.
            let on = [vandq_u32(live[0], vtstq_u32(i[0], i[0])), vandq_u32(live[1], vtstq_u32(i[1], i[1]))];
            let before = [vandq_u32(vsubq_u32(i[0], one), on[0]), vandq_u32(vsubq_u32(i[1], one), on[1])];
            let whole = c + 8 <= n;
            let (f, st) = split(&slots.read(&before, &on, c.checked_sub(1).filter(|_| whole)));
            let mut res = [vdupq_n_u32(NONE); 2];
            // A lane's sibling `j'` in the chunk: its lane (else its own), and the lanes waiting on one.
            let mut link = [l0, l1];
            let mut wait = [vdupq_n_u32(0); 2];
            let mut sib = [0u32; 8];
            let mut out_of = [0u32; 8];
            for r in 0..2 {
                let found = vandq_u32(on[r], vceqzq_u32(vandq_u32(f[r], one)));
                res[r] = vbslq_u32(found, before[r], res[r]);
                let mut go = vbicq_u32(on[r], found);
                go = vbicq_u32(go, vcgeq_u32(vsubq_u32(st[r], one), i[r]));
                let jp = vsubq_u32(i[r], st[r]);
                let up = vandq_u32(go, vtstq_u32(f[r], vdupq_n_u32(2)));
                res[r] = vbslq_u32(up, jp, res[r]);
                // A previous sibling, not the start: `j'`'s answer.
                go = vandq_u32(vbicq_u32(go, up), vtstq_u32(jp, jp));
                let inside = vandq_u32(go, vcgeq_u32(jp, vdupq_n_u32(c)));
                link[r] = vbslq_u32(inside, vsubq_u32(jp, vdupq_n_u32(c)), link[r]);
                wait[r] = inside;
                vst1q_u32(sib.as_mut_ptr().add(4 * r), jp);
                vst1q_u32(out_of.as_mut_ptr().add(4 * r), vbicq_u32(go, inside));
            }
            // A `j'` before the chunk: its answer made.
            if vmaxvq_u32(vorrq_u32(vld1q_u32(out_of.as_ptr()), vld1q_u32(out_of.as_ptr().add(4)))) != 0 {
                let mut r8 = [0u32; 8];
                vst1q_u32(r8.as_mut_ptr(), res[0]);
                vst1q_u32(r8.as_mut_ptr().add(4), res[1]);
                for k in 0..8 {
                    if out_of[k] != 0 {
                        r8[k] = *out.get_unchecked(sib[k] as usize);
                    }
                }
                res = [vld1q_u32(r8.as_ptr()), vld1q_u32(r8.as_ptr().add(4))];
            }
            // A `j'` in the chunk: its lane's answer, by pointer jumping (a chain of at most 7 links).
            if vmaxvq_u32(vorrq_u32(wait[0], wait[1])) != 0 {
                for _ in 0..3 {
                    let (r2, w2, l2) = (take8(&res, &link), take8(&wait, &link), take8(&link, &link));
                    for r in 0..2 {
                        res[r] = vbslq_u32(wait[r], r2[r], res[r]);
                        link[r] = vbslq_u32(wait[r], l2[r], link[r]);
                        wait[r] = vandq_u32(wait[r], w2[r]);
                    }
                }
                debug_assert!(vmaxvq_u32(vorrq_u32(wait[0], wait[1])) == 0);
            }
            if whole {
                vst1q_u32(out.as_mut_ptr().add(c as usize), res[0]);
                vst1q_u32(out.as_mut_ptr().add(c as usize + 4), res[1]);
            } else {
                let mut r8 = [0u32; 8];
                vst1q_u32(r8.as_mut_ptr(), res[0]);
                vst1q_u32(r8.as_mut_ptr().add(4), res[1]);
                out[c as usize..n as usize].copy_from_slice(&r8[..(n - c) as usize]);
            }
            c += 8;
        }
    }
}

/// Side list `k`'s values (`reads::f16`'s `side`) of the nodes `i` of the lanes `need` marks, the others 0. A node's
/// entry is its block's run of offsets (the low bytes of the block's listed nodes, ascending) searched for its low
/// byte: the host's read halves the run, a chain of dependent loads; here a lane's index is the count of the run's
/// offsets below its byte, the run (at most a block's 256, in practice a few to 32) compared 16 at a time and summed,
/// no load waiting on another, the lanes' runs independent too. A run that would read past the tape's end (or past
/// its part's, no tape's) is the host's read.
///
/// # Safety
///
/// As `parent_all`'s.
#[inline(never)]
unsafe fn side8(t: &[u8], v: &rerail_clif::view::TapeView, k: usize, i: &[u32; 8], need: &[u32; 8]) -> [u64; 8] {
    use rerail_clif::view::{F16_BLOCK as BLOCK, F16_PART as PART, F16_SUB as SUB};
    let at = |base: u32, off: u32, n: usize| -> &[u8] {
        let from = base as usize + off as usize;
        t.get(from..from + n).unwrap_or_else(|| panic!("format 16: a read past the tape's end"))
    };
    let u16_at = |base, off| u32::from(u16::from_le_bytes(at(base, off, 2).try_into().unwrap()));
    let u32_at = |base, off| u32::from_le_bytes(at(base, off, 4).try_into().unwrap());
    let (list, parts) = (v.side[k], v.parts[k]);
    let blocks = v.nodes.wrapping_add(BLOCK - 1) / BLOCK;
    let vw = (v.widths >> (16 + 4 * k)) & 15;
    let m = if vw >= 8 { u64::MAX } else { (1u64 << (8 * vw)) - 1 };
    const IOTA: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
    let mut out = [0u64; 8];
    for l in 0..8 {
        if need[l] == 0 {
            continue;
        }
        let p = i[l] / PART;
        let start = u32_at(parts, p * 4);
        let limit = u32_at(parts, p * 4 + 4);
        let nsub = SUB.min(blocks.wrapping_sub(p * SUB));
        let j = (i[l] / BLOCK) % SUB;
        let lo = if j == 0 { 0 } else { u16_at(list, start.wrapping_add((j - 1) * 2).min(limit)) };
        let hi = u16_at(list, start.wrapping_add(j * 2).min(limit));
        let offsets = start.wrapping_add(nsub.wrapping_mul(2));
        let count = u16_at(list, start.wrapping_add(nsub.wrapping_sub(1).wrapping_mul(2)).min(limit));
        let len = hi.saturating_sub(lo);
        let from = list as usize + offsets as usize + lo as usize;
        if offsets.wrapping_add(hi) > limit || from + len.next_multiple_of(16) as usize > t.len() {
            out[l] = rerail_tape_format::reads::f16::slot(t, v, i[l]);
            continue;
        }
        // The run's offsets below the node's low byte, 16 at a time (a piece's lanes past the run's end off).
        let mut below = 0u32;
        unsafe {
            let (low, iota) = (vdupq_n_u8((i[l] % BLOCK) as u8), vld1q_u8(IOTA.as_ptr()));
            let mut q = 0;
            while q < len {
                let keys = vld1q_u8(t.as_ptr().add(from + q as usize));
                let inside = vcltq_u8(iota, vdupq_n_u8((len - q).min(16) as u8));
                below += u32::from(vaddvq_u8(vandq_u8(vandq_u8(vcltq_u8(keys, low), inside), vdupq_n_u8(1))));
                q += 16;
            }
        }
        // The list's values follow the part's offsets.
        let a = offsets.wrapping_add(count).wrapping_add((lo + below).wrapping_mul(vw)).min(limit);
        out[l] = u64::from_le_bytes(at(list, a, 8).try_into().unwrap()) & m;
    }
    out
}

/// Every node's slot (`reads::f16::slot`) by the 8-lane reader: its runs, its gathers, its side-list count search
/// (its tests' entry: every escaped slot of a tape read once).
///
/// # Safety
///
/// As `parent_all`'s; `out` holds the tape's nodes.
pub unsafe fn slots_all(t: &[u8], v: &rerail_clif::view::TapeView, out: &mut [u64]) {
    let n = v.nodes;
    unsafe {
        let mut slots = Slots::new(t, v);
        let (l0, l1) = (vld1q_u32(LANES8.as_ptr()), vld1q_u32(LANES8.as_ptr().add(4)));
        let mut c = 0;
        while c < n {
            let at = [vaddq_u32(vdupq_n_u32(c), l0), vaddq_u32(vdupq_n_u32(c), l1)];
            let on = [vcltq_u32(at[0], vdupq_n_u32(n)), vcltq_u32(at[1], vdupq_n_u32(n))];
            // A lane past the end reads node 0's.
            let at = [vandq_u32(at[0], on[0]), vandq_u32(at[1], on[1])];
            let s = slots.read(&at, &on, Some(c).filter(|_| c + 8 <= n));
            let mut s8 = [0u64; 8];
            for r in 0..4 {
                vst1q_u64(s8.as_mut_ptr().add(2 * r), s[r]);
            }
            let k = (n - c).min(8) as usize;
            out[c as usize..c as usize + k].copy_from_slice(&s8[..k]);
            c += 8;
        }
    }
}
