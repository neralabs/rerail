// The preprocessor's searches over its sorted and scanned stores: a lower bound, and an output
// element's item. The scans, segmented scans and sorts are the persistent grid's (`ukl/grid.h`,
// decision 0460 §4), no longer kernels of their own here.

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

// For measurements: the grid's sort alone (`ukl_grid_sort_u64`), `n` keys by their low `bits`.
kernel void pb_sort_alone(device uint* ctl [[buffer(0)]], device ulong* keys [[buffer(1)]], device uint* values [[buffer(2)]],
                          device ulong* keys2 [[buffer(3)]], device uint* values2 [[buffer(4)]], device uint* hist [[buffer(5)]],
                          device uint* sums [[buffer(6)]], constant uint& n [[buffer(7)]], constant uint& bits [[buffer(8)]],
                          uint lid [[thread_position_in_threadgroup]], uint group [[threadgroup_position_in_grid]],
                          uint groups [[threadgroups_per_grid]], uint lsz [[threads_per_threadgroup]]) {
    threadgroup uint flag;
    threadgroup uint scratch[UKL_TG_SCRATCH];
    threadgroup uint big[UKL_RADIX_GROUPS * UKL_RADIX_DIGITS + 3u * UKL_RADIX_DIGITS];
    UklGrid g;
    g.ctl = ctl; g.groups = groups; g.group = group; g.lid = lid; g.lsz = lsz;
    UklGridSortLocal sl;
    sl.counts = big; sl.h = big + UKL_RADIX_GROUPS * UKL_RADIX_DIGITS;
    sl.before = sl.h + UKL_RADIX_DIGITS; sl.base = sl.before + UKL_RADIX_DIGITS;
    ukl_grid_sort_u64(g, n, bits, true, (UKL_COHERENT uint64_t*)keys, (UKL_COHERENT uint32_t*)values, (UKL_COHERENT uint64_t*)keys2,
                      (UKL_COHERENT uint32_t*)values2, (UKL_COHERENT uint32_t*)hist, (UKL_COHERENT uint32_t*)sums, sl, scratch, &flag);
}

// For measurements: `n` grid barriers, then (`what` 1) `n` device fences, (2) `n` threadgroup
// barriers with device scope, (3) `n` dependent coherent loads by thread 0.
kernel void pb_barriers_alone(device uint* ctl [[buffer(0)]], device uint* data [[buffer(1)]], constant uint& n [[buffer(2)]],
                              constant uint& what [[buffer(3)]], uint lid [[thread_position_in_threadgroup]],
                              uint group [[threadgroup_position_in_grid]], uint groups [[threadgroups_per_grid]],
                              uint lsz [[threads_per_threadgroup]]) {
    threadgroup uint flag;
    UklGrid g;
    g.ctl = ctl; g.groups = groups; g.group = group; g.lid = lid; g.lsz = lsz;
    if (what == 0u) { for (uint k = 0u; k < n; k++) if (!ukl_grid_barrier(g, &flag)) return; }
    if (what == 1u) { for (uint k = 0u; k < n; k++) { ukl_fence_device(); data[lid] += k; } }
    if (what == 2u) { for (uint k = 0u; k < n; k++) { ukl_tg_barrier_device(); data[lid] += k; } }
    if (what == 3u && lid == 0u) {
        UKL_COHERENT uint* c = (UKL_COHERENT uint*)data;
        uint at = 0u;
        for (uint k = 0u; k < n; k++) at = c[at & 255u] + k;
        data[256] = at;
    }
    if (what == 4u && lid == 0u) {
        uint at = 0u;
        for (uint k = 0u; k < n; k++) at = data[at & 255u] + k;
        data[256] = at;
    }
}
