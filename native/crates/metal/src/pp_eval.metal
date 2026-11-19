// The preprocessor's evaluator over tapes (decision 0436): plain MSL passes over a unit's files,
// each a source's bytes and its preprocessor tape (a v9np image, read in place: no node is
// decoded into another array). The host loads the files into `data` and lists them in `files`;
// every size a pass needs after that is made on the device (counts, a prefix sum, and the next
// dispatch's arguments).
#include <metal_stdlib>
using namespace metal;

// A file of the unit: its source's bytes and its tape's, at byte offsets in `data` (16-aligned).
struct PpFile { uint src, src_len, tape, tape_len; };

// The tape's kinds the passes read (`rerail_c_pp::tape`).
constant uint K_IDENTIFIER = 28u;

// --- A v9np image read in place (`rerail_tape_format::View`) -------------------------------------

constant uint HEADER_BYTES = 64u;
constant uint STRIDE = 16u;

struct Tape {
    device const uchar* d;
    uint n, codes;
    uint ws, wo, wt;
    bool from_start;
    uint code_col, size_col, base_col, off_col, tail_col, table;
};

inline uint pad16(uint x) { return (x + 15u) & ~15u; }
inline uint rd16(device const uchar* d, uint a) { return *(device const ushort*)(d + a); }
inline uint rd32(device const uchar* d, uint a) { return *(device const uint*)(d + a); }

// The image at `at` of `data` (16-aligned, so every column is aligned to its width).
inline Tape tape_at(device const uchar* data, uint at) {
    Tape t;
    t.d = data + at;
    t.n = rd32(t.d, 24u);
    t.codes = rd32(t.d, 32u);
    t.ws = t.d[9];
    t.wo = t.d[10];
    t.wt = t.d[11] & 0x7fu;
    t.from_start = (t.d[11] & 0x80u) != 0u;
    uint c = HEADER_BYTES;
    t.code_col = c; c = pad16(c + 2u * t.n);
    t.size_col = c; c = pad16(c + t.ws * t.n);
    t.base_col = c; c = pad16(c + 4u * ((t.n + STRIDE - 1u) / STRIDE));
    t.off_col = c; c = pad16(c + t.wo * t.n);
    t.tail_col = c; c = pad16(c + t.wt * t.n);
    t.table = c;
    return t;
}

inline uint cell(Tape t, uint col, uint w, uint i) { return w == 2u ? rd16(t.d, col + 2u * i) : rd32(t.d, col + 4u * i); }

// Node `i`'s key: `kf | flags << 16`. A code is 6 bytes where it has no supertype classes, as a
// preprocessor tape's never has (`pp_check` says so).
inline uint node_key(Tape t, uint i) {
    uint e = t.table + 6u * rd16(t.d, t.code_col + 2u * i);
    return rd16(t.d, e) | uint(t.d[e + 2u]) << 16;
}
inline uint node_kind(Tape t, uint i) { return node_key(t, i) & 0x3ffu; }
inline uint node_size(Tape t, uint i) { return max(cell(t, t.size_col, t.ws, i), 1u); }
inline uint node_start(Tape t, uint i) { return rd32(t.d, t.base_col + 4u * (i / STRIDE)) + cell(t, t.off_col, t.wo, i); }
inline uint node_end(Tape t, uint i) {
    uint from = t.from_start ? node_start(t, i) : node_start(t, i + node_size(t, i) - 1u);
    return from + cell(t, t.tail_col, t.wt, i);
}

// --- The unit's nodes: a count a file, a prefix sum, a node's file --------------------------------

// Each file's node count, and whether its codes are all 6 bytes (bit 31 of `bad` where not).
kernel void pp_counts(device const PpFile* files [[buffer(0)]], device const uchar* data [[buffer(1)]],
                      device uint* counts [[buffer(2)]], device atomic_uint* bad [[buffer(3)]],
                      constant uint& n_files [[buffer(4)]], uint f [[thread_position_in_grid]]) {
    if (f >= n_files) return;
    Tape t = tape_at(data, files[f].tape);
    counts[f] = t.n;
    // The code table's length is 6 bytes a code exactly when no code has a class.
    uint code_bytes = rd32(t.d, 40u);
    if (code_bytes != 6u * t.codes) atomic_fetch_or_explicit(bad, 1u << 31, memory_order_relaxed);
}

// The exclusive prefix sum of `counts` (each file's first node in the unit), the total, and the
// arguments of a dispatch of one thread a node in groups of 256. One threadgroup of 1024 threads:
// each sums a run of files, the runs' sums are scanned, and each run is written.
kernel void pp_scan(device const uint* counts [[buffer(0)]], device uint* bases [[buffer(1)]],
                    device uint* total [[buffer(2)]], device uint* args [[buffer(3)]],
                    constant uint& n [[buffer(4)]], uint l [[thread_position_in_threadgroup]],
                    uint lane [[thread_index_in_simdgroup]], uint sg [[simdgroup_index_in_threadgroup]]) {
    threadgroup uint sums[32];
    uint per = (n + 1023u) / 1024u;
    uint a = min(l * per, n), b = min(a + per, n);
    uint s = 0u;
    for (uint f = a; f < b; f++) s += counts[f];
    uint before = simd_prefix_exclusive_sum(s);
    if (lane == 31u) sums[sg] = before + s;
    threadgroup_barrier(mem_flags::mem_threadgroup);
    if (sg == 0u) sums[lane] = simd_prefix_exclusive_sum(sums[lane]);
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint run = sums[sg] + before;
    for (uint f = a; f < b; f++) {
        bases[f] = run;
        run += counts[f];
    }
    if (l == 1023u) {
        total[0] = run;
        args[0] = (run + 255u) / 256u;
        args[1] = 1u;
        args[2] = 1u;
    }
}

// The file of the unit's node `g`: the last whose first node is at or before it.
inline uint file_of(device const uint* bases, uint n_files, uint g) {
    uint lo = 0u, hi = n_files;
    while (hi - lo > 1u) {
        uint mid = (lo + hi) / 2u;
        if (bases[mid] <= g) lo = mid; else hi = mid;
    }
    return lo;
}

// --- Names -------------------------------------------------------------------------------------

// The 8 bytes of `data` at byte `a`, as a little-endian word.
inline ulong word_at(device const ulong* d, uint a) {
    uint q = a >> 3, r = (a & 7u) * 8u;
    ulong lo = d[q];
    return r == 0u ? lo : (lo >> r) | (d[q + 1u] << (64u - r));
}

// A name's key: its bytes a word at a time (`rerail_metal::pp_eval::name_key`).
inline ulong name_key(device const ulong* d, uint a, uint len) {
    ulong h = 0x9e3779b97f4a7c15ul ^ ulong(len);
    for (uint k = 0u; k < len; k += 8u) {
        ulong w = word_at(d, a + k);
        uint left = len - k;
        if (left < 8u) w &= (1ul << (left * 8u)) - 1ul;
        h = (h ^ w) * 0xff51afd7ed558ccdul;
        h ^= h >> 32;
    }
    h *= 0xc4ceb9fe1a85ec53ul;
    return h ^ (h >> 29);
}

// Each identifier leaf's key, 0 for every other node: one thread a node of the unit.
kernel void pp_names(device const PpFile* files [[buffer(0)]], device const uchar* data [[buffer(1)]],
                     device const uint* bases [[buffer(2)]], device const uint* total [[buffer(3)]],
                     device ulong* keys [[buffer(4)]], constant uint& n_files [[buffer(5)]],
                     uint g [[thread_position_in_grid]]) {
    if (g >= total[0]) return;
    uint f = file_of(bases, n_files, g);
    Tape t = tape_at(data, files[f].tape);
    uint i = g - bases[f];
    ulong key = 0ul;
    if (node_kind(t, i) == K_IDENTIFIER) {
        uint s = node_start(t, i);
        key = name_key((device const ulong*)data, files[f].src + s, node_end(t, i) - s);
    }
    keys[g] = key;
}

// For tests: each node of the unit as the reader gives it (key, start, end, size).
kernel void pp_nodes(device const PpFile* files [[buffer(0)]], device const uchar* data [[buffer(1)]],
                     device const uint* bases [[buffer(2)]], device const uint* total [[buffer(3)]],
                     device uint4* out [[buffer(4)]], constant uint& n_files [[buffer(5)]],
                     uint g [[thread_position_in_grid]]) {
    if (g >= total[0]) return;
    uint f = file_of(bases, n_files, g);
    Tape t = tape_at(data, files[f].tape);
    uint i = g - bases[f];
    out[g] = uint4(node_key(t, i), node_start(t, i), node_end(t, i), node_size(t, i));
}
