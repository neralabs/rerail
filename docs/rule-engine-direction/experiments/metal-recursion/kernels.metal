#include <metal_stdlib>
using namespace metal;

// Class bits per kind: 1 = target, 2 = prune (stopBy: do not descend below it).
constant uint T = 1u, P = 2u;

struct Params { uint rootCount; uint exists; uint stackCap; uint queueCap; uint spinCap; uint itemCount; };

// (a) Recursive formulation: per-root explicit stack over a CSR child list (pointer chasing).
kernel void a_stack(device const ushort* kind [[buffer(0)]],
                    device const uint* childStart [[buffer(1)]],
                    device const uint* childList [[buffer(2)]],
                    device const uchar* cls [[buffer(3)]],
                    device const uint* roots [[buffer(4)]],
                    constant Params& p [[buffer(5)]],
                    device uint* out [[buffer(6)]],
                    device atomic_uint* overflow [[buffer(7)]],
                    uint tid [[thread_position_in_grid]]) {
    if (tid >= p.rootCount) return;
    uint r = roots[tid];
    uint stack[64];
    uint sp = 0, hits = 0;
    for (uint c = childStart[r]; c < childStart[r + 1]; ++c) {
        if (sp == p.stackCap) { atomic_fetch_add_explicit(overflow, 1u, memory_order_relaxed); out[tid] = 0xFFFFFFFFu; return; }
        stack[sp++] = childList[c];
    }
    while (sp > 0) {
        uint n = stack[--sp];
        uint k = cls[kind[n]];
        if (k & T) { hits++; if (p.exists) break; }
        if (k & P) continue;
        for (uint c = childStart[n]; c < childStart[n + 1]; ++c) {
            if (sp == p.stackCap) { atomic_fetch_add_explicit(overflow, 1u, memory_order_relaxed); out[tid] = 0xFFFFFFFFu; return; }
            stack[sp++] = childList[c];
        }
    }
    out[tid] = hits;
}

// (b) Interval formulation over the preorder tape: subtree(r) = [r+1, r+size[r]); a pruned
// node skips its subtree by adding its size. No stack, no recursion, sequential reads.
kernel void b_interval(device const ushort* kind [[buffer(0)]],
                       device const uint* size [[buffer(1)]],
                       device const uchar* cls [[buffer(3)]],
                       device const uint* roots [[buffer(4)]],
                       constant Params& p [[buffer(5)]],
                       device uint* out [[buffer(6)]],
                       uint tid [[thread_position_in_grid]]) {
    if (tid >= p.rootCount) return;
    uint r = roots[tid];
    uint end = r + size[r], hits = 0;
    uint j = r + 1;
    while (j < end) {
        uint k = cls[kind[j]];
        if (k & T) { hits++; if (p.exists) break; }
        j += (k & P) ? size[j] : 1u;
    }
    out[tid] = hits;
}

// (b2) Posting formulation: sorted preorder positions of target nodes; a subtree is a range,
// so EXISTS/COUNT without pruning is two binary searches.
static uint lower_bound(device const uint* a, uint n, uint x) {
    uint lo = 0, hi = n;
    while (lo < hi) { uint mid = (lo + hi) >> 1; if (a[mid] < x) lo = mid + 1; else hi = mid; }
    return lo;
}
kernel void b2_posting(device const uint* size [[buffer(1)]],
                       device const uint* postings [[buffer(2)]],
                       device const uint* roots [[buffer(4)]],
                       constant Params& p [[buffer(5)]],
                       device uint* out [[buffer(6)]],
                       uint tid [[thread_position_in_grid]]) {
    if (tid >= p.rootCount) return;
    uint r = roots[tid];
    uint lo = lower_bound(postings, p.itemCount, r + 1);
    uint hi = lower_bound(postings, p.itemCount, r + size[r]);
    out[tid] = p.exists ? (hi > lo ? 1u : 0u) : hi - lo;
}

// (b3) Owner column: for each target posting, its nearest strict ancestor in the owner set
// (prune kinds plus root kinds), computed once per batch by a bounded parent walk.
kernel void b3_owner(device const ushort* kind [[buffer(0)]],
                     device const uint* parent [[buffer(1)]],
                     device const uchar* cls [[buffer(3)]],
                     device const uint* postings [[buffer(2)]],
                     constant Params& p [[buffer(5)]],
                     device uint* owner [[buffer(6)]],
                     uint tid [[thread_position_in_grid]]) {
    if (tid >= p.itemCount) return;
    uint n = parent[postings[tid]];
    while (n != 0xFFFFFFFFu && !(cls[kind[n]] & P)) n = parent[n];
    owner[tid] = n;
}
kernel void b3_query(device const uint* size [[buffer(1)]],
                     device const uint* postings [[buffer(2)]],
                     device const uint* owner [[buffer(3)]],
                     device const uint* roots [[buffer(4)]],
                     constant Params& p [[buffer(5)]],
                     device uint* out [[buffer(6)]],
                     uint tid [[thread_position_in_grid]]) {
    if (tid >= p.rootCount) return;
    uint r = roots[tid];
    uint lo = lower_bound(postings, p.itemCount, r + 1);
    uint hi = lower_bound(postings, p.itemCount, r + size[r]);
    uint hits = 0;
    for (uint i = lo; i < hi; ++i) if (owner[i] == r) { hits++; if (p.exists) break; }
    out[tid] = hits;
}

// (c) Persistent threads with a device-side work queue of (root slot, node) items.
kernel void c_persistent(device const ushort* kind [[buffer(0)]],
                         device const uint* size [[buffer(1)]],
                         device atomic_uint* qRoot [[buffer(2)]],
                         device const uchar* cls [[buffer(3)]],
                         device atomic_uint* qNode [[buffer(4)]],
                         constant Params& p [[buffer(5)]],
                         device atomic_uint* out [[buffer(6)]],
                         device atomic_uint* ctr [[buffer(7)]], // 0 head, 1 tail, 2 done, 3 overflow, 4 timeout
                         uint tid [[thread_position_in_grid]]) {
    uint spins = 0;
    while (true) {
        uint i = atomic_fetch_add_explicit(&ctr[0], 1u, memory_order_relaxed);
        if (i >= p.queueCap) return;
        uint slot, n;
        while (true) {
            n = atomic_load_explicit(&qNode[i], memory_order_relaxed);
            slot = atomic_load_explicit(&qRoot[i], memory_order_relaxed);
            if (n != 0xFFFFFFFFu && slot != 0xFFFFFFFFu) break;
            uint tail = atomic_load_explicit(&ctr[1], memory_order_relaxed);
            uint done = atomic_load_explicit(&ctr[2], memory_order_relaxed);
            if (i >= tail && done == tail) return;
            if (++spins > p.spinCap) { atomic_store_explicit(&ctr[4], 1u, memory_order_relaxed); return; }
        }
        bool expand = !(p.exists && atomic_load_explicit(&out[slot], memory_order_relaxed) != 0u);
        uint end = n + size[n];
        for (uint j = n + 1; expand && j < end; j += size[j]) {
            uint k = cls[kind[j]];
            if (k & T) { atomic_fetch_add_explicit(&out[slot], 1u, memory_order_relaxed); if (p.exists) break; }
            if (k & P) continue;
            if (size[j] == 1u) continue; // a leaf has nothing to expand
            uint w = atomic_fetch_add_explicit(&ctr[1], 1u, memory_order_relaxed);
            if (w >= p.queueCap) { atomic_store_explicit(&ctr[3], 1u, memory_order_relaxed); continue; }
            atomic_store_explicit(&qRoot[w], slot, memory_order_relaxed);
            atomic_store_explicit(&qNode[w], j, memory_order_relaxed);
        }
        atomic_fetch_add_explicit(&ctr[2], 1u, memory_order_relaxed);
    }
}

// (d) Level-synchronous frontier waves with GPU-driven (indirect) dispatch: no CPU round trip.
struct Indirect { uint x, y, z; };
kernel void d_prep(device atomic_uint* counts [[buffer(0)]],
                   device Indirect* args [[buffer(1)]],
                   constant uint& wave [[buffer(2)]]) {
    uint n = atomic_load_explicit(&counts[wave], memory_order_relaxed);
    args[wave] = Indirect{ (n + 255u) / 256u, 1u, 1u };
}
kernel void d_wave(device const ushort* kind [[buffer(0)]],
                   device const uint* size [[buffer(1)]],
                   device const uint2* curr [[buffer(2)]],
                   device const uchar* cls [[buffer(3)]],
                   device uint2* next [[buffer(4)]],
                   constant Params& p [[buffer(5)]],
                   device atomic_uint* out [[buffer(6)]],
                   device atomic_uint* counts [[buffer(7)]],
                   constant uint& wave [[buffer(8)]],
                   uint tid [[thread_position_in_grid]]) {
    uint count = atomic_load_explicit(&counts[wave], memory_order_relaxed);
    if (tid >= count) return;
    uint2 item = curr[tid];
    if (p.exists && atomic_load_explicit(&out[item.x], memory_order_relaxed) != 0u) return;
    uint n = item.y, end = n + size[n];
    for (uint j = n + 1; j < end; j += size[j]) {
        uint k = cls[kind[j]];
        if (k & T) { atomic_fetch_add_explicit(&out[item.x], 1u, memory_order_relaxed); if (p.exists) return; }
        if (k & P) continue;
        if (size[j] == 1u) continue; // a leaf has nothing to expand
        uint w = atomic_fetch_add_explicit(&counts[wave + 1], 1u, memory_order_relaxed);
        if (w < p.queueCap) next[w] = uint2(item.x, j);
        else atomic_fetch_add_explicit(&counts[63], 1u, memory_order_relaxed);
    }
}

// (e) inside/ancestor test: bounded parent walk (depth <= tape max depth), stops at a prune kind.
kernel void e_inside(device const ushort* kind [[buffer(0)]],
                     device const uint* parent [[buffer(1)]],
                     device const uchar* cls [[buffer(3)]],
                     device const uint* roots [[buffer(4)]],
                     constant Params& p [[buffer(5)]],
                     device uint* out [[buffer(6)]],
                     uint tid [[thread_position_in_grid]]) {
    if (tid >= p.rootCount) return;
    uint n = parent[roots[tid]], hit = 0;
    while (n != 0xFFFFFFFFu) {
        uint k = cls[kind[n]];
        if (k & T) { hit = 1; break; }
        if (k & P) break;
        n = parent[n];
    }
    out[tid] = hit;
}

// (f) Nearest strict ancestor whose kind prunes (a generic per-document "scope owner" column),
// computed node-parallel by a bounded parent walk. NONE where no ancestor prunes.
kernel void f_prune_anc(device const ushort* kind [[buffer(0)]],
                        device const uint* parent [[buffer(1)]],
                        device const uchar* cls [[buffer(3)]],
                        constant uint& n [[buffer(5)]],
                        device uint* anc [[buffer(6)]],
                        uint tid [[thread_position_in_grid]]) {
    if (tid >= n) return;
    uint a = parent[tid];
    while (a != 0xFFFFFFFFu && !(cls[kind[a]] & P)) a = parent[a];
    anc[tid] = a;
}
// SIMD-group cooperative interval scan: one 32-lane SIMD group per root, lanes stride the
// interval; a node is reachable iff its nearest pruning ancestor is the root or above it.
kernel void f_simd_scan(device const ushort* kind [[buffer(0)]],
                        device const uint* size [[buffer(1)]],
                        device const uint* anc [[buffer(2)]],
                        device const uchar* cls [[buffer(3)]],
                        device const uint* roots [[buffer(4)]],
                        constant Params& p [[buffer(5)]],
                        device uint* out [[buffer(6)]],
                        uint gid [[thread_position_in_grid]],
                        uint lane [[thread_index_in_simdgroup]]) {
    uint slot = gid / 32u;
    if (slot >= p.rootCount) return;
    uint r = roots[slot];
    uint end = r + size[r], hits = 0;
    for (uint base = r + 1; base < end; base += 32u) {
        uint j = base + lane;
        bool hit = false;
        if (j < end) {
            uint a = anc[j];
            hit = (cls[kind[j]] & T) && (a == 0xFFFFFFFFu || a <= r);
        }
        if (p.exists) { if (simd_any(hit)) { hits = 1; break; } }
        else hits += simd_sum(hit ? 1u : 0u);
    }
    if (lane == 0) out[slot] = hits;
}
