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
