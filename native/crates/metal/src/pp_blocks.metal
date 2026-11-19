// The building blocks of the preprocessor's grid passes (decision 0448 §3): a scan, a segmented
// scan, a stable radix sort, a search and an expand, every one sized on the device. A count is
// read from a device buffer (`n[at]`); a plan kernel turns it into the levels' counts and the
// indirect arguments of the dispatches after it, so the host encodes them without reading it.
//
// Every kernel runs groups of PB_TG = 256 threads (8 SIMD groups of 32), one thread an element,
// or four in a scan's block. Threadgroup memory: at most 8.3 KB (the sort's per-SIMD counts).

constant uint PB_TG = 256u;
// Elements of a scan's block: 4 a thread.
constant uint PB_BLOCK = 1024u;

// --- Plans ---------------------------------------------------------------------------------------

// Indirect arguments of `groups` threadgroups at `args[at..at + 3]`.
inline void pb_set_args(device uint* args, uint at, uint groups) {
    args[at] = groups;
    args[at + 1u] = 1u;
    args[at + 2u] = 1u;
}

// A scan's plan for `n[at]` elements, at most 2^30:
//   [0] n0, [1] n1 = its blocks, [2] n2 = their blocks;
//   [4..7) level 0's groups (n1), [7..10) level 1's (n2), [10..13) level 2's (1);
//   [13..16) the add-back of level 0 (n0 / 256), [16..19) of level 1 (n1 / 256).
kernel void pb_scan_plan(device const uint* n [[buffer(0)]], device uint* plan [[buffer(1)]],
                         constant uint& at [[buffer(2)]], uint i [[thread_position_in_grid]]) {
    if (i != 0u) return;
    uint n0 = n[at], n1 = (n0 + PB_BLOCK - 1u) / PB_BLOCK, n2 = (n1 + PB_BLOCK - 1u) / PB_BLOCK;
    plan[0] = n0;
    plan[1] = n1;
    plan[2] = n2;
    pb_set_args(plan, 4u, n1);
    pb_set_args(plan, 7u, n2);
    pb_set_args(plan, 10u, 1u);
    pb_set_args(plan, 13u, (n0 + PB_TG - 1u) / PB_TG);
    pb_set_args(plan, 16u, (n1 + PB_TG - 1u) / PB_TG);
}

// The arguments of one thread an element over `n[at]` elements, groups of 256, at `args[0..3)`.
kernel void pb_args(device const uint* n [[buffer(0)]], device uint* args [[buffer(1)]],
                    constant uint& at [[buffer(2)]], uint i [[thread_position_in_grid]]) {
    if (i != 0u) return;
    pb_set_args(args, 0u, (n[at] + PB_TG - 1u) / PB_TG);
}

// --- The scan --------------------------------------------------------------------------------------

// The exclusive scan of `v` across the threadgroup; `total` gets its sum. `sh` holds 33 uints.
inline uint pb_tg_exscan(uint v, threadgroup uint* sh, uint lane, uint sg, uint nsg, thread uint& total) {
    uint x = simd_prefix_exclusive_sum(v);
    if (lane == 31u) sh[sg] = x + v;
    threadgroup_barrier(mem_flags::mem_threadgroup);
    if (sg == 0u) {
        uint s = lane < nsg ? sh[lane] : 0u;
        uint ps = simd_prefix_exclusive_sum(s);
        if (lane < nsg) sh[lane] = ps;
        if (lane == 31u) sh[32] = ps + s;
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint r = x + sh[sg];
    total = sh[32];
    return r;
}

// A level of the exclusive scan: block `b` scans its PB_BLOCK elements of `in` (`plan[level]`
// of them) into `out`, and writes its sum to `sums[b]`.
kernel void pb_scan_block(device const uint* in [[buffer(0)]], device uint* out [[buffer(1)]],
                          device uint* sums [[buffer(2)]], device const uint* plan [[buffer(3)]],
                          constant uint& level [[buffer(4)]], uint b [[threadgroup_position_in_grid]],
                          uint t [[thread_index_in_threadgroup]], uint lane [[thread_index_in_simdgroup]],
                          uint sg [[simdgroup_index_in_threadgroup]], uint nsg [[simdgroups_per_threadgroup]]) {
    threadgroup uint sh[33];
    uint n = plan[level];
    uint base = b * PB_BLOCK + t * 4u;
    uint v[4];
    uint s = 0u;
    for (uint q = 0u; q < 4u; q++) {
        uint i = base + q;
        v[q] = i < n ? in[i] : 0u;
        s += v[q];
    }
    uint total;
    uint p = pb_tg_exscan(s, sh, lane, sg, nsg, total);
    for (uint q = 0u; q < 4u; q++) {
        if (base + q < n) out[base + q] = p;
        p += v[q];
    }
    if (t == 0u) sums[b] = total;
}

// The add-back: each of `plan[level]` elements adds its block's prefix.
kernel void pb_scan_add(device uint* out [[buffer(0)]], device const uint* prefix [[buffer(1)]],
                        device const uint* plan [[buffer(2)]], constant uint& level [[buffer(3)]],
                        uint i [[thread_position_in_grid]]) {
    if (i < plan[level]) out[i] += prefix[i / PB_BLOCK];
}

// --- The segmented scan ----------------------------------------------------------------------------

// The segmented inclusive scan's operator: 0 add, 1 max.
constant uint PB_OP [[function_constant(0)]];

inline uint pb_op(uint a, uint b) { return PB_OP == 0u ? a + b : max(a, b); }

// `a` then `b` (a segment head in `b` cuts what comes before it).
struct Seg { uint f, v; };
inline Seg pb_seg(Seg a, Seg b) {
    Seg r;
    r.f = a.f | b.f;
    r.v = b.f != 0u ? b.v : pb_op(a.v, b.v);
    return r;
}

// The inclusive segmented scan of `x` across the SIMD group, by a ladder of shuffles.
inline Seg pb_simd_seg(Seg x, uint lane) {
    for (uint d = 1u; d < 32u; d <<= 1u) {
        Seg o;
        o.f = simd_shuffle_up(x.f, d);
        o.v = simd_shuffle_up(x.v, d);
        if (lane >= d) x = pb_seg(o, x);
    }
    return x;
}

// A level of the segmented scan: block `b`'s PB_BLOCK elements (`plan[level]` of them), each
// a value and a head flag, scanned inclusive into `out`; `seen[i]` whether a head is at or
// before `i` in its block; the block's aggregate to `agg_f[b]`, `agg_v[b]`.
kernel void pb_seg_block(device const uint* in [[buffer(0)]], device const uint* heads [[buffer(1)]],
                         device uint* out [[buffer(2)]], device uint* seen [[buffer(3)]],
                         device uint* agg_v [[buffer(4)]], device uint* agg_f [[buffer(5)]],
                         device const uint* plan [[buffer(6)]], constant uint& level [[buffer(7)]],
                         uint b [[threadgroup_position_in_grid]], uint t [[thread_index_in_threadgroup]],
                         uint lane [[thread_index_in_simdgroup]], uint sg [[simdgroup_index_in_threadgroup]],
                         uint nsg [[simdgroups_per_threadgroup]]) {
    threadgroup uint sh_f[33], sh_v[33];
    uint n = plan[level];
    uint base = b * PB_BLOCK + t * 4u;
    Seg local[4];
    Seg run = {0u, 0u};
    for (uint q = 0u; q < 4u; q++) {
        uint i = base + q;
        Seg e = {i < n ? heads[i] : 0u, i < n ? in[i] : 0u};
        run = pb_seg(run, e);
        local[q] = run;
    }
    Seg incl = pb_simd_seg(run, lane);
    Seg excl;
    excl.f = simd_shuffle_up(incl.f, 1u);
    excl.v = simd_shuffle_up(incl.v, 1u);
    if (lane == 0u) excl = Seg{0u, 0u};
    if (lane == 31u) {
        sh_f[sg] = incl.f;
        sh_v[sg] = incl.v;
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    if (sg == 0u) {
        Seg s = lane < nsg ? Seg{sh_f[lane], sh_v[lane]} : Seg{0u, 0u};
        Seg si = pb_simd_seg(s, lane);
        Seg se;
        se.f = simd_shuffle_up(si.f, 1u);
        se.v = simd_shuffle_up(si.v, 1u);
        if (lane == 0u) se = Seg{0u, 0u};
        if (lane < nsg) {
            sh_f[lane] = se.f;
            sh_v[lane] = se.v;
        }
        if (lane == nsg - 1u) {
            sh_f[32] = si.f;
            sh_v[32] = si.v;
        }
    }
    threadgroup_barrier(mem_flags::mem_threadgroup);
    Seg p = pb_seg(Seg{sh_f[sg], sh_v[sg]}, excl);
    for (uint q = 0u; q < 4u; q++) {
        uint i = base + q;
        if (i < n) {
            Seg r = pb_seg(p, local[q]);
            out[i] = r.v;
            seen[i] = r.f;
        }
    }
    if (t == 0u) {
        agg_f[b] = sh_f[32];
        agg_v[b] = sh_v[32];
    }
}

// The add-back: each of `plan[level]` elements past block 0 with no head before it in its block
// takes the blocks before it (`prefix`, their inclusive scan).
kernel void pb_seg_add(device uint* out [[buffer(0)]], device const uint* seen [[buffer(1)]],
                       device const uint* prefix [[buffer(2)]], device const uint* plan [[buffer(3)]],
                       constant uint& level [[buffer(4)]], uint i [[thread_position_in_grid]]) {
    if (i >= plan[level]) return;
    uint b = i / PB_BLOCK;
    if (b > 0u && seen[i] == 0u) out[i] = pb_op(prefix[b - 1u], out[i]);
}

// --- The radix sort ----------------------------------------------------------------------------------

// A sort's plan for `n[at]` elements: [0] n, [1] tiles (of 256), [2] the histogram's length
// (256 a tile); [4..7) one group a tile.
kernel void pb_sort_plan(device const uint* n [[buffer(0)]], device uint* plan [[buffer(1)]],
                         constant uint& at [[buffer(2)]], uint i [[thread_position_in_grid]]) {
    if (i != 0u) return;
    uint m = n[at], tiles = (m + PB_TG - 1u) / PB_TG;
    plan[0] = m;
    plan[1] = tiles;
    plan[2] = tiles * 256u;
    pb_set_args(plan, 4u, tiles);
}

// Each element's own index, its payload before the sort.
kernel void pb_iota(device uint* vals [[buffer(0)]], device const uint* plan [[buffer(1)]],
                    uint i [[thread_position_in_grid]]) {
    if (i < plan[0]) vals[i] = i;
}

// Tile `g`'s count of each digit at `shift`, at `hist[digit * tiles + g]`.
kernel void pb_hist(device const ulong* keys [[buffer(0)]], device uint* hist [[buffer(1)]],
                    device const uint* plan [[buffer(2)]], constant uint& shift [[buffer(3)]],
                    uint g [[threadgroup_position_in_grid]], uint t [[thread_index_in_threadgroup]]) {
    threadgroup atomic_uint cnt[256];
    atomic_store_explicit(&cnt[t], 0u, memory_order_relaxed);
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint i = g * PB_TG + t;
    if (i < plan[0]) atomic_fetch_add_explicit(&cnt[uint(keys[i] >> shift) & 255u], 1u, memory_order_relaxed);
    threadgroup_barrier(mem_flags::mem_threadgroup);
    hist[t * plan[1] + g] = atomic_load_explicit(&cnt[t], memory_order_relaxed);
}

// The stable scatter of tile `g` by its digit at `shift`: each element's place is its digit's
// first in the scanned histogram, plus the earlier SIMD groups' elements of that digit, plus
// its rank among its SIMD group's (its peers by 8 ballots).
kernel void pb_scatter(device const ulong* keys [[buffer(0)]], device const uint* vals [[buffer(1)]],
                       device ulong* keys_out [[buffer(2)]], device uint* vals_out [[buffer(3)]],
                       device const uint* hist [[buffer(4)]], device const uint* plan [[buffer(5)]],
                       constant uint& shift [[buffer(6)]], uint g [[threadgroup_position_in_grid]],
                       uint t [[thread_index_in_threadgroup]], uint lane [[thread_index_in_simdgroup]],
                       uint sg [[simdgroup_index_in_threadgroup]]) {
    threadgroup uint cnt[8][256];
    for (uint k = 0u; k < 8u; k++) cnt[k][t] = 0u;
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint i = g * PB_TG + t;
    bool on = i < plan[0];
    ulong key = on ? keys[i] : 0ul;
    uint d = uint(key >> shift) & 255u;
    uint peers = uint(static_cast<simd_vote::vote_t>(simd_ballot(on)));
    for (uint bit = 0u; bit < 8u; bit++) {
        bool set = ((d >> bit) & 1u) != 0u;
        uint b = uint(static_cast<simd_vote::vote_t>(simd_ballot(on && set)));
        peers &= set ? b : ~b;
    }
    uint rank = popcount(peers & ((1u << lane) - 1u));
    if (on && ctz(peers) == lane) cnt[sg][d] = popcount(peers);
    threadgroup_barrier(mem_flags::mem_threadgroup);
    if (!on) return;
    uint before = 0u;
    for (uint k = 0u; k < sg; k++) before += cnt[k][d];
    uint at = hist[d * plan[1] + g] + before + rank;
    keys_out[at] = key;
    vals_out[at] = vals[i];
}

// --- Search and expand -------------------------------------------------------------------------------

// The first place in `sorted[0..n)` whose key is at least `q`.
inline uint pb_lower_bound(device const ulong* sorted, uint n, ulong q) {
    uint lo = 0u, hi = n;
    for (uint k = 0u; k < 32u && lo < hi; k++) {
        uint mid = (lo + hi) / 2u;
        if (sorted[mid] < q) lo = mid + 1u; else hi = mid;
    }
    return lo;
}

// Each of `nq[at_q]` queries' lower bound in `sorted` (`ns[at_s]` keys).
kernel void pb_search(device const ulong* sorted [[buffer(0)]], device const uint* ns [[buffer(1)]],
                      device const ulong* queries [[buffer(2)]], device const uint* nq [[buffer(3)]],
                      device uint* out [[buffer(4)]], constant uint& at_s [[buffer(5)]],
                      constant uint& at_q [[buffer(6)]], uint i [[thread_position_in_grid]]) {
    if (i < nq[at_q]) out[i] = pb_lower_bound(sorted, ns[at_s], queries[i]);
}

// The item that output element `j` comes from: the last of `n` items whose first (`offsets`,
// an exclusive scan of their counts) is at or before `j`.
inline uint pb_item_of(device const uint* offsets, uint n, uint j) {
    uint lo = 0u, hi = n;
    for (uint k = 0u; k < 32u && hi - lo > 1u; k++) {
        uint mid = (lo + hi) / 2u;
        if (offsets[mid] <= j) lo = mid; else hi = mid;
    }
    return lo;
}

// Each of `total[0]` output elements' item and its rank in the item (`n[at]` items).
kernel void pb_expand(device const uint* offsets [[buffer(0)]], device const uint* n [[buffer(1)]],
                      device const uint* total [[buffer(2)]], device uint* item [[buffer(3)]],
                      device uint* rank [[buffer(4)]], constant uint& at [[buffer(5)]],
                      uint j [[thread_position_in_grid]]) {
    if (j >= total[0]) return;
    uint k = pb_item_of(offsets, n[at], j);
    item[j] = k;
    rank[j] = j - offsets[k];
}
