//! t39 step 5: the bulk primitives as Metal kernels, on each candidate's one-buffer image read in place
//! (`inplace::GpuImage`, the kernels' readers in `inplace_gpu::SOURCE`). One thread a group of 64 nodes, by
//! preorder rank:
//!
//! - `match`: the group's 64-bit mask of S-nodes (each code read and looked up in the set);
//! - `nearest` (and outermost): the thread climbs from its group's first node to the S-ancestors open there,
//!   its carry, then sweeps its 64 nodes with that stack. XF reads an S-node's subtree end. BE reads no end:
//!   it walks the parentheses from the group's first open (found by a select over the rank directory), and an
//!   S-node closes when the depth comes back to its own. BE's climb is a search a level, so BE's carry is a
//!   scan instead (`b_local`, `b_up`, `b_down`, `b_resolve`): each group's walk is a stack transformer,
//!   composed up and down a 64-ary tree; outermost then follows each answer's chain of S-nodes (`b_outer`);
//! - `has`: from each S-node of the group, a climb that sets each ancestor's flag by an atomic exchange and
//!   stops at a flag already set (another climb is doing the rest), so the work is the nodes marked.
//!
//! Every answer is checked against the tree.

use crate::inplace::{B, EfEf, ForLen, GpuImage, PredGap, Reader, XF};
use metal::{Device, MTLResourceOptions, MTLSize};

const KERNELS: &str = r#"
// ---- t39 step 5: bulk primitives ----
#define STACK 64
#define NONE 0xffffffffu
#define BARGS device const uchar* buf [[buffer(0)]], constant uint* P [[buffer(1)]], device const uchar* tabs [[buffer(2)]], \
    device const uchar* set [[buffer(3)]], device ulong* masks [[buffer(4)]], device uint* out [[buffer(5)]], \
    constant uint& mode [[buffer(6)]], device atomic_uint* bad [[buffer(7)]], uint g [[thread_position_in_grid]]

kernel void xf_match(BARGS) {
    Rd r = { buf, P, 0 };
    XFC x = rd_xf(r);
    uint k0 = g * 64;
    if (k0 >= x.n) return;
    ulong m = 0;
    uint hi = min(64u, x.n - k0);
    for (uint j = 0; j < hi; j++) { m |= ulong(set[x.code.get(k0 + j)]) << j; }
    masks[g] = m;
}

// mode 0: nearest, 1: outermost.
kernel void xf_nearest(BARGS) {
    Rd r = { buf, P, 0 };
    XFC x = rd_xf(r);
    uint k0 = g * 64;
    if (k0 >= x.n) return;
    uint st[STACK], se[STACK];
    int sp = 0;
    // The carry: the S-ancestors of the group's first node, innermost first, then turned outermost first.
    ulong p1 = x.parent1(k0);
    while (p1 != 0) {
        uint p = uint(p1 - 1);
        if (set[x.code.get(p)]) {
            if (sp == STACK) { atomic_fetch_add_explicit(bad, 1u, memory_order_relaxed); return; }
            st[sp] = p; se[sp] = x.end(p); sp++;
        }
        p1 = x.parent1(p);
    }
    for (int i = 0; i < sp / 2; i++) {
        uint a = st[i]; st[i] = st[sp - 1 - i]; st[sp - 1 - i] = a;
        uint b = se[i]; se[i] = se[sp - 1 - i]; se[sp - 1 - i] = b;
    }
    ulong m = masks[g];
    uint hi = min(64u, x.n - k0);
    for (uint j = 0; j < hi; j++) {
        uint k = k0 + j;
        while (sp > 0 && se[sp - 1] <= k) sp--;
        out[k] = sp == 0 ? NONE : (mode == 0 ? st[sp - 1] : st[0]);
        if ((m >> j) & 1) {
            if (sp == STACK) { atomic_fetch_add_explicit(bad, 1u, memory_order_relaxed); return; }
            st[sp] = k; se[sp] = x.end(k); sp++;
        }
    }
}

kernel void xf_has(BARGS) {
    Rd r = { buf, P, 0 };
    XFC x = rd_xf(r);
    uint k0 = g * 64;
    if (k0 >= x.n) return;
    device atomic_uint* flag = (device atomic_uint*)out;
    ulong m = masks[g];
    while (m != 0) {
        uint s = k0 + ctz(m);
        m &= m - 1;
        ulong p1 = x.parent1(s);
        while (p1 != 0) {
            uint p = uint(p1 - 1);
            if (atomic_exchange_explicit(&flag[p], 1u, memory_order_relaxed) == 1u) break;
            p1 = x.parent1(p);
        }
    }
}

// ---- BE: groups by rank over the parentheses ----
// The position of the open of rank k: the superblock by binary search over the rank directory, then the word,
// then the one within it.
inline uint b_select1(BC b, uint k, Tab t) {
    uint nsb = (b.nbits + 511) / 512;
    uint lo = 0, hi = nsb;
    while (hi - lo > 1) { uint mid = (lo + hi) / 2; if (b.sb_abs[mid] <= k) lo = mid; else hi = mid; }
    uint r = k - b.sb_abs[lo];
    uint w = lo * 8;
    for (;;) { uint c = popcount(b.bits[w]); if (r < c) break; r -= c; w++; }
    return w * 64 + select_in_word(b.bits[w], r, t);
}

kernel void b_match(BARGS) {
    Rd r = { buf, P, 0 };
    BC b = rd_b(r);
    uint k0 = g * 64;
    if (k0 >= b.n) return;
    ulong m = 0;
    uint hi = min(64u, b.n - k0);
    for (uint j = 0; j < hi; j++) { m |= ulong(set[b.code.get(k0 + j)]) << j; }
    masks[g] = m;
}

kernel void b_nearest(BARGS) {
    Rd r = { buf, P, 0 };
    BC b = rd_b(r);
    Tab tb = { tabs };
    uint k0 = g * 64;
    if (k0 >= b.n) return;
    uint q = b_select1(b, k0, tb);
    uint st[STACK];
    int sd[STACK];
    int sp = 0;
    // The carry: S-ancestors with their depths relative to the group's first node (its parent at −1).
    int d = -1;
    uint e = b.enclose(q, tb);
    while (e != NONE) {
        uint k = b.rank1(e);
        if (set[b.code.get(k)]) {
            if (sp == STACK) { atomic_fetch_add_explicit(bad, 1u, memory_order_relaxed); return; }
            st[sp] = k; sd[sp] = d; sp++;
        }
        d--;
        e = b.enclose(e, tb);
    }
    for (int i = 0; i < sp / 2; i++) {
        uint a = st[i]; st[i] = st[sp - 1 - i]; st[sp - 1 - i] = a;
        int c = sd[i]; sd[i] = sd[sp - 1 - i]; sd[sp - 1 - i] = c;
    }
    ulong m = masks[g];
    uint hi = min(64u, b.n - k0);
    // The depth of the next open; an S-node opened at depth d closes when the depth comes back to d.
    d = 0;
    uint c = 0;
    while (c < hi) {
        ulong word = b.bits[q >> 6] >> (q & 63);
        uint left = 64 - (q & 63);
        for (uint i = 0; i < left && c < hi; i++, q++) {
            if ((word >> i) & 1) {
                uint k = k0 + c;
                out[k] = sp == 0 ? NONE : (mode == 0 ? st[sp - 1] : st[0]);
                if ((m >> c) & 1) {
                    if (sp == STACK) { atomic_fetch_add_explicit(bad, 1u, memory_order_relaxed); return; }
                    st[sp] = k; sd[sp] = d; sp++;
                }
                d++;
                c++;
            } else {
                d--;
                if (sp > 0 && sd[sp - 1] == d) sp--;
            }
        }
    }
}

kernel void b_has(BARGS) {
    Rd r = { buf, P, 0 };
    BC b = rd_b(r);
    Tab tb = { tabs };
    uint k0 = g * 64;
    if (k0 >= b.n) return;
    device atomic_uint* flag = (device atomic_uint*)out;
    ulong m = masks[g];
    if (m == 0) return;
    // The S-nodes' positions: from the group's first open, counting opens.
    uint q = b_select1(b, k0, tb);
    uint c = 0;
    while (m != 0) {
        uint want = ctz(m);
        m &= m - 1;
        while (c < want || !b.is_open(q)) { if (b.is_open(q)) c++; q++; }
        uint e = b.enclose(q, tb);
        while (e != NONE) {
            if (atomic_exchange_explicit(&flag[b.rank1(e)], 1u, memory_order_relaxed) == 1u) break;
            e = b.enclose(e, tb);
        }
    }
}

// ---- BE: nearest as a scan with block carries ----
// A group's walk is a stack transformer: the lowest level it reaches (m, from its start, at most 0), its net
// level change, and the S-nodes still open at its end (innermost last, levels from its start). Transformers
// compose; a 64-ary tree of them, up then down, gives each group the S-nodes open at its start (levels from
// the forest's start). The group then answers the nodes it could not answer alone.
#define K 16
struct Sum { int m; int delta; uint cnt; uint flag; uint rank[K]; int depth[K]; };
inline void sum_id(thread Sum& s) { s.m = 0; s.delta = 0; s.cnt = 0; s.flag = 0; }
// a = a then b.
inline void compose(thread Sum& a, device const Sum& b) {
    int lim = a.delta + b.m;
    uint c = 0;
    for (uint i = 0; i < a.cnt; i++) { if (a.depth[i] < lim) { a.rank[c] = a.rank[i]; a.depth[c] = a.depth[i]; c++; } }
    for (uint i = 0; i < b.cnt; i++) {
        if (c == K) { for (uint j = 1; j < K; j++) { a.rank[j - 1] = a.rank[j]; a.depth[j - 1] = a.depth[j]; } c = K - 1; a.flag = 1; }
        a.rank[c] = b.rank[i]; a.depth[c] = b.depth[i] + a.delta; c++;
    }
    a.flag |= b.flag;
    a.m = min(a.m, lim);
    a.delta += b.delta;
    a.cnt = c;
}

#define SARGS device const uchar* buf [[buffer(0)]], constant uint* P [[buffer(1)]], device const uchar* tabs [[buffer(2)]], \
    device const uchar* set [[buffer(3)]], device ulong* masks [[buffer(4)]], device uint* out [[buffer(5)]], \
    constant uint& mode [[buffer(6)]], device atomic_uint* bad [[buffer(7)]], device Sum* sums [[buffer(8)]], \
    device Sum* excl [[buffer(9)]], device ushort* aux [[buffer(10)]], device uint* out2 [[buffer(11)]], \
    constant uint* lv [[buffer(12)]], constant uint& level [[buffer(13)]], uint g [[thread_position_in_grid]]

// The walk alone: each node's answer if an S-node of the group is open over it (else NONE, and in aux the
// lowest level since the group's start, negated), and the group's transformer.
kernel void b_local(SARGS) {
    Rd r = { buf, P, 0 };
    BC b = rd_b(r);
    Tab tb = { tabs };
    uint k0 = g * 64;
    if (k0 >= b.n) return;
    uint q = b_select1(b, k0, tb);
    uint st[K];
    int sd[K];
    int sp = 0, d = 0, mn = 0;
    uint flag = 0;
    ulong m = masks[g];
    uint hi = min(64u, b.n - k0);
    uint c = 0;
    // The group's opens, then the closes up to the next group's first open.
    while (q < b.nbits) {
        ulong word = b.bits[q >> 6] >> (q & 63);
        uint left = 64 - (q & 63);
        bool done = false;
        for (uint i = 0; i < left && q < b.nbits; i++, q++) {
            if ((word >> i) & 1) {
                if (c == hi) { done = true; break; }
                uint k = k0 + c;
                out[k] = sp == 0 ? NONE : st[sp - 1];
                aux[k] = ushort(-mn);
                if ((m >> c) & 1) {
                    if (sp == K) { for (int j = 1; j < K; j++) { st[j - 1] = st[j]; sd[j - 1] = sd[j]; } sp = K - 1; flag = 1; }
                    st[sp] = k; sd[sp] = d; sp++;
                }
                d++;
                c++;
            } else {
                d--;
                mn = min(mn, d);
                if (sp > 0 && sd[sp - 1] == d) sp--;
            }
        }
        if (done) break;
    }
    device Sum& s = sums[lv[1] + g];
    s.m = mn; s.delta = d; s.cnt = uint(sp); s.flag = flag;
    for (int i = 0; i < sp; i++) { s.rank[i] = st[i]; s.depth[i] = sd[i]; }
}

// lv: [levels, then (offset, count) a level]. Up: a parent of `level + 1` composes its (up to) 64 children.
kernel void b_up(SARGS) {
    uint off = lv[1 + 2 * level], cnt = lv[2 + 2 * level], poff = lv[3 + 2 * level], pcnt = lv[4 + 2 * level];
    if (g >= pcnt) return;
    Sum a;
    sum_id(a);
    for (uint c = g * 64; c < min(cnt, g * 64 + 64); c++) compose(a, sums[off + c]);
    sums[poff + g] = a;
}

// Down: a parent of `level + 1` knows what comes before it; each child gets that, then its siblings before it.
kernel void b_down(SARGS) {
    uint off = lv[1 + 2 * level], cnt = lv[2 + 2 * level], poff = lv[3 + 2 * level], pcnt = lv[4 + 2 * level];
    if (g >= pcnt) return;
    Sum a;
    if (level + 2 == lv[0]) { sum_id(a); } else { a = excl[poff + g]; }
    for (uint c = g * 64; c < min(cnt, g * 64 + 64); c++) { excl[off + c] = a; compose(a, sums[off + c]); }
}

// Each node with no S-node of its group over it: the innermost S-node open at the group's start whose level
// is below the lowest the walk reached before the node.
kernel void b_resolve(SARGS) {
    Rd r = { buf, P, 0 };
    BC b = rd_b(r);
    uint k0 = g * 64, n = b.n;
    if (k0 >= n) return;
    device const Sum& e = excl[lv[1] + g];
    if (e.flag) atomic_fetch_add_explicit(bad, 1u, memory_order_relaxed);
    for (uint j = 0; j < 64; j++) {
        uint k = k0 + j;
        if (k >= n) break;
        if (out[k] != NONE) continue;
        int thr = e.delta - int(aux[k]);
        for (int i = int(e.cnt) - 1; i >= 0; i--) { if (e.depth[i] < thr) { out[k] = e.rank[i]; break; } }
    }
}

// outermost: follow the S-nodes' own answers to the end of the chain.
kernel void b_outer(SARGS) {
    Rd r = { buf, P, 0 };
    BC b = rd_b(r);
    uint k0 = g * 64;
    if (k0 >= b.n) return;
    for (uint j = 0; j < min(64u, b.n - k0); j++) {
        uint k = k0 + j;
        uint a = out[k];
        if (a != NONE) { while (out[a] != NONE) a = out[a]; }
        out2[k] = a;
    }
}
"#;

pub struct Row {
    /// GPU ns a node, best of 8 warm dispatches: match, nearest, outermost, has (each with its match).
    pub ns: [f64; 4],
    /// BE: nearest with the carry found by climbing (the first form), against the scan in `ns`.
    pub no_carry: Option<f64>,
    pub wrong: usize,
}

/// One candidate, one set: the kernels `<pre>_match`, `<pre>_nearest`, `<pre>_has` over its image.
pub fn run(pre: &str, image: &GpuImage, n: usize, set: &[bool], truth: (&[u32], &[u32], &[bool])) -> Result<Row, String> {
    let device = Device::system_default().ok_or("no Metal device")?;
    let src = format!("{}{}", crate::inplace_gpu::SOURCE, KERNELS);
    let lib = device.new_library_with_source(&src, &metal::CompileOptions::new()).map_err(|e| e.to_string())?;
    let pipe = |name: &str| -> Result<metal::ComputePipelineState, String> {
        let f = lib.get_function(&format!("{pre}_{name}"), None).map_err(|e| e.to_string())?;
        device.new_compute_pipeline_state_with_function(&f).map_err(|e| e.to_string())
    };
    let (pm, pn, ph) = (pipe("match")?, pipe("nearest")?, pipe("has")?);
    let opts = MTLResourceOptions::StorageModeShared;
    let buf = |b: &[u8]| device.new_buffer_with_data(b.as_ptr().cast(), b.len().max(1) as u64, opts);
    let u32s = |v: &[u32]| -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() };
    let mut tabs = crate::inplace::byte_tables_bytes();
    tabs.extend(crate::inplace::select8_bytes());
    let set_bytes: Vec<u8> = set.iter().map(|&b| u8::from(b)).collect();
    let (bimg, bpar, btab, bset) = (buf(&image.bytes), buf(&u32s(&image.params)), buf(&tabs), buf(&set_bytes));
    let groups = n.div_ceil(64);
    let bmask = device.new_buffer((groups * 8).max(8) as u64, opts);
    let bout = device.new_buffer((n * 4).max(4) as u64, opts);
    let bbad = device.new_buffer(4, opts);
    // The scan's levels: groups, then a parent a 64, up to one (at least two levels).
    let mut lv = vec![0u32];
    let (mut off, mut cnt) = (0usize, groups);
    loop {
        lv.extend([off as u32, cnt as u32]);
        off += cnt;
        if cnt == 1 && lv.len() > 3 {
            break;
        }
        cnt = cnt.div_ceil(64);
    }
    let levels = (lv.len() - 1) / 2;
    lv[0] = levels as u32;
    const SUM: usize = 16 + 8 * 16;
    let (bsum, bexcl) = (device.new_buffer((off * SUM) as u64, opts), device.new_buffer((off * SUM) as u64, opts));
    let baux = device.new_buffer((n * 2).max(2) as u64, opts);
    let bout2 = device.new_buffer((n * 4).max(4) as u64, opts);
    let blv = buf(&u32s(&lv));
    let queue = device.new_command_queue();
    // A stage: pipeline, mode, level, threads.
    type Stage<'a> = (&'a metal::ComputePipelineState, u32, u32, usize);
    // Runs the stages in one command buffer; `clear` zeroes the answer first. The encoder is serial (the
    // default), so each dispatch sees the one before's writes.
    let time = |stages: &[Stage], clear: bool| -> f64 {
        let mut best = f64::MAX;
        // Warm until the clocks are up (the first dispatches run slow), then the best of 8.
        let mut last = f64::MAX;
        for round in 0..48 {
            let cmd = queue.new_command_buffer();
            if clear {
                let bl = cmd.new_blit_command_encoder();
                bl.fill_buffer(&bout, metal::NSRange::new(0, (n * 4) as u64), 0);
                bl.end_encoding();
            }
            let enc = cmd.new_compute_command_encoder();
            for &(p, mode, level, threads) in stages {
                enc.set_compute_pipeline_state(p);
                for (i, b) in [&bimg, &bpar, &btab, &bset, &bmask, &bout].into_iter().enumerate() {
                    enc.set_buffer(i as u64, Some(b), 0);
                }
                enc.set_bytes(6, 4, (&mode as *const u32).cast());
                enc.set_buffer(7, Some(&bbad), 0);
                for (i, b) in [&bsum, &bexcl, &baux, &bout2, &blv].into_iter().enumerate() {
                    enc.set_buffer(8 + i as u64, Some(b), 0);
                }
                enc.set_bytes(13, 4, (&level as *const u32).cast());
                let w = p.thread_execution_width();
                enc.dispatch_threads(MTLSize::new(threads.max(1) as u64, 1, 1), MTLSize::new(w, 1, 1));
            }
            enc.end_encoding();
            cmd.commit();
            cmd.wait_until_completed();
            let t = crate::gpu::command_seconds(cmd);
            if round >= 8 {
                best = best.min(t);
                if round >= 16 && t > 0.97 * last && best < f64::MAX {
                    break;
                }
            }
            last = t;
        }
        best * 1e9 / n as f64
    };
    // SAFETY: shared storage, and the GPU is done with it.
    let got = || unsafe { std::slice::from_raw_parts(bout.contents().cast::<u32>(), n) }.to_vec();
    let mut wrong = 0usize;
    let g = groups;
    let t_match = time(&[(&pm, 0, 0, g)], false);
    let t_has = time(&[(&pm, 0, 0, g), (&ph, 0, 0, g)], true);
    wrong += got().iter().zip(truth.2).filter(|&(a, &b)| (*a == 1) != b).count();
    let (t_near, t_outer, no_carry);
    if pre == "b" {
        // The scan: the walk, up the levels, down them, then each group's leftovers; outermost follows chains.
        let (pl, pu, pd, pr, po) = (pipe("local")?, pipe("up")?, pipe("down")?, pipe("resolve")?, pipe("outer")?);
        let mut st: Vec<Stage> = vec![(&pm, 0, 0, g), (&pl, 0, 0, g)];
        for l in 0..levels - 1 {
            st.push((&pu, 0, l as u32, lv[4 + 2 * l] as usize));
        }
        for l in (0..levels - 1).rev() {
            st.push((&pd, 0, l as u32, lv[4 + 2 * l] as usize));
        }
        st.push((&pr, 0, 0, g));
        t_near = time(&st, false);
        wrong += got().iter().zip(truth.0).filter(|(a, b)| a != b).count();
        st.push((&po, 0, 0, g));
        t_outer = time(&st, false);
        // SAFETY: shared storage, and the GPU is done with it.
        let o2 = unsafe { std::slice::from_raw_parts(bout2.contents().cast::<u32>(), n) };
        wrong += o2.iter().zip(truth.1).filter(|(a, b)| a != b).count();
        // The first form, climbing for the carry, for comparison.
        no_carry = Some(time(&[(&pm, 0, 0, g), (&pn, 0, 0, g)], false));
        wrong += got().iter().zip(truth.0).filter(|(a, b)| a != b).count();
    } else {
        t_near = time(&[(&pm, 0, 0, g), (&pn, 0, 0, g)], false);
        wrong += got().iter().zip(truth.0).filter(|(a, b)| a != b).count();
        t_outer = time(&[(&pm, 0, 0, g), (&pn, 1, 0, g)], false);
        wrong += got().iter().zip(truth.1).filter(|(a, b)| a != b).count();
        no_carry = None;
    }
    // SAFETY: as above.
    wrong += unsafe { *bbad.contents().cast::<u32>() } as usize;
    Ok(Row { ns: [t_match, t_near, t_outer, t_has], no_carry, wrong })
}

/// XF, MF and BE on one forest, for one set: (name, GPU row, CPU ns a node for nearest, outermost, has from
/// the S-nodes, best of 3).
pub fn all(tree: &crate::tree::Tree, codes: &crate::inplace::Codes, set: &[bool]) -> Result<Vec<(&'static str, Row, [f64; 3])>, String> {
    let n = tree.len();
    let code: Vec<u32> = tree.nodes.iter().map(|x| codes.ids[&(x.kind, x.field, x.flags, x.classes.clone())]).collect();
    let inset: Vec<bool> = code.iter().map(|&c| set[c as usize]).collect();
    let (tn, to, th) = (crate::prim::truth::nearest(tree, &inset), crate::prim::truth::outermost(tree, &inset), crate::prim::truth::has(tree, &inset));
    fn cpu<R: Reader>(r: &R, set: &[bool], n: usize, t: (&[u32], &[u32], &[bool])) -> ([f64; 3], usize) {
        let best = |f: &mut dyn FnMut()| -> f64 {
            (0..3).map(|_| {
                let s = std::time::Instant::now();
                f();
                s.elapsed().as_secs_f64()
            }).fold(f64::MAX, f64::min) * 1e9 / n as f64
        };
        let (mut a, mut h) = (Vec::new(), Vec::new());
        let mut wrong = 0;
        let x0 = best(&mut || crate::prim::axis_groups(r, set, false, &mut a));
        wrong += a.iter().zip(t.0).filter(|(x, y)| x != y).count();
        let x1 = best(&mut || crate::prim::axis_groups(r, set, true, &mut a));
        wrong += a.iter().zip(t.1).filter(|(x, y)| x != y).count();
        let x2 = best(&mut || crate::prim::has_groups(r, set, &mut h));
        wrong += h.iter().zip(t.2).filter(|(x, y)| x != y).count();
        ([x0, x1, x2], wrong)
    }
    let t = (&tn[..], &to[..], &th[..]);
    let mut rows = Vec::new();
    let xf = XF::<ForLen>::new(tree, codes);
    let (c, w) = cpu(&xf, set, n, t);
    let mut row = run("xf", &xf.gpu_image(), n, set, t)?;
    row.wrong += w;
    rows.push(("XF", row, c));
    drop(xf);
    let mf = XF::<PredGap>::new(tree, codes);
    let (c, w) = cpu(&mf, set, n, t);
    let mut row = run("xf", &mf.gpu_image(), n, set, t)?;
    row.wrong += w;
    rows.push(("MF", row, c));
    drop(mf);
    let be = B::<EfEf>::new(tree, codes);
    let (c, w) = cpu(&be, set, n, t);
    let mut row = run("b", &be.gpu_image(), n, set, t)?;
    row.wrong += w;
    rows.push(("BE", row, c));
    Ok(rows)
}
