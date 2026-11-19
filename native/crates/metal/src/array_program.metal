// Array programs on the device (decision 0430): each operation of `rerail_compiler::array` as
// data-parallel kernels over columns of 64-bit integers. `array_program.rs` lays the program out
// as launches; the interpreter (`ArrayProgram::values`) is the meaning each kernel keeps.
//
// - Elementwise operations (map, select, iota, fill, copy, gather): a thread an element; an
//   operand of length 1 is broadcast.
// - Scans: a tile of 256 scanned in threadgroup memory (segment flags as pairs), the tiles'
//   totals scanned the same way, then added to every element before its tile's first flag.
// - Sorts: a stable radix sort, a byte a pass: each tile sorted by its byte (eight one-bit
//   splits), the tiles' counts scanned digit by digit, each element written at its digit's place.
// - Searches: a bounded binary search a query. Scatters: a stable sort by index, a segmented
//   scan of each index's values, and its last written.
//
// An error is the first failing operation's index and its reason, kept by an atomic minimum.

#include <metal_stdlib>
using namespace metal;

// A launch's parameters (`Launch`).
struct L {
    uint n;
    uint la;
    uint lb;
    uint lc;
    int op;
    int flag;
    uint at;
    uint tiles;
    long c;
    long d;
};

constant uint TILE = 256;

static void fail(device atomic_uint* err, uint at, uint why) {
    atomic_fetch_min_explicit(err, (at << 8) | why, memory_order_relaxed);
}

static long map_op(int op, long a, long b) {
    ulong ua = as_type<ulong>(a), ub = as_type<ulong>(b);
    switch (op) {
        case 0: return as_type<long>(ua + ub);
        case 1: return as_type<long>(ua - ub);
        case 2: return as_type<long>(ua * ub);
        case 3: return a & b;
        case 4: return a | b;
        case 5: return a ^ b;
        case 6: return min(a, b);
        case 7: return max(a, b);
        case 8: return a == b ? 1 : 0;
        case 9: return a != b ? 1 : 0;
        case 10: return a < b ? 1 : 0;
        case 11: return a <= b ? 1 : 0;
        case 12: return as_type<long>(ua << (uint(b) & 63));
        default: return a >> (uint(b) & 63);
    }
}

// A combine: 0 set (the later), 1 add, 2 max, 3 min.
static long combine(int op, long a, long b) {
    switch (op) {
        case 0: return b;
        case 1: return as_type<long>(as_type<ulong>(a) + as_type<ulong>(b));
        case 2: return max(a, b);
        default: return min(a, b);
    }
}

static long identity(int op) {
    switch (op) {
        case 2: return LONG_MIN;
        case 3: return LONG_MAX;
        default: return 0;
    }
}

kernel void a_map(device const long* x [[buffer(0)]], device const long* y [[buffer(1)]], device long* out [[buffer(2)]],
                  constant L& p [[buffer(3)]], uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    out[j] = map_op(p.op, x[p.la == 1 ? 0 : j], y[p.lb == 1 ? 0 : j]);
}

kernel void a_select(device const long* m [[buffer(0)]], device const long* x [[buffer(1)]], device const long* y [[buffer(2)]],
                     device long* out [[buffer(3)]], constant L& p [[buffer(4)]], uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    out[j] = m[p.la == 1 ? 0 : j] != 0 ? x[p.lb == 1 ? 0 : j] : y[p.lc == 1 ? 0 : j];
}

kernel void a_iota(device long* out [[buffer(0)]], constant L& p [[buffer(1)]], uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    out[j] = long(j);
}

kernel void a_fill(device long* out [[buffer(0)]], device const long* v [[buffer(1)]], constant L& p [[buffer(2)]],
                   uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    out[j] = v[0];
}

// `src` (broadcast where `la` is 1) into `out` from `d`.
kernel void a_copy(device const long* src [[buffer(0)]], device long* out [[buffer(1)]], constant L& p [[buffer(2)]],
                   uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    out[p.d + j] = src[p.la == 1 ? 0 : j];
}

kernel void a_gather(device const long* s [[buffer(0)]], device const long* idx [[buffer(1)]], device long* out [[buffer(2)]],
                     device atomic_uint* err [[buffer(3)]], constant L& p [[buffer(4)]], uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    long i = idx[j];
    if (i < 0 || i >= long(p.la)) {
        fail(err, p.at, 1);
        out[j] = 0;
    } else {
        out[j] = s[i];
    }
}

// A tile's inclusive segmented scan: each element's value and whether a flag is at or before it
// in its tile; the tile's total and whether it holds a flag. `flag`: the values have flags.
kernel void a_scan_tile(device const long* x [[buffer(0)]], device const long* f [[buffer(1)]], device long* out [[buffer(2)]],
                        device long* fl [[buffer(3)]], device long* agg [[buffer(4)]], device long* aggf [[buffer(5)]],
                        constant L& p [[buffer(6)]], uint j [[thread_position_in_grid]], uint lid [[thread_position_in_threadgroup]],
                        uint t [[threadgroup_position_in_grid]]) {
    threadgroup long sv[TILE];
    threadgroup uchar sf[TILE];
    long v = j < p.n ? x[j] : identity(p.op);
    uchar g = (j < p.n && p.flag != 0 && f[j] != 0) ? 1 : 0;
    sv[lid] = v;
    sf[lid] = g;
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for (uint s = 1; s < TILE; s <<= 1) {
        bool has = lid >= s;
        long pv = has ? sv[lid - s] : 0;
        uchar pf = has ? sf[lid - s] : 0;
        threadgroup_barrier(mem_flags::mem_threadgroup);
        if (has) {
            if (g == 0) v = combine(p.op, pv, v);
            g |= pf;
            sv[lid] = v;
            sf[lid] = g;
        }
        threadgroup_barrier(mem_flags::mem_threadgroup);
    }
    if (j < p.n) {
        out[j] = v;
        fl[j] = g;
    }
    if (lid == TILE - 1) {
        agg[t] = v;
        aggf[t] = g;
    }
}

// The tiles before an element's, where no flag in its tile is at or before it.
kernel void a_scan_fix(device long* out [[buffer(0)]], device const long* fl [[buffer(1)]], device const long* aggi [[buffer(2)]],
                       constant L& p [[buffer(3)]], uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    uint t = j / TILE;
    if (t > 0 && fl[j] == 0) out[j] = combine(p.op, aggi[t - 1], out[j]);
}

// An exclusive scan from the inclusive: the identity at a flag and at the first element.
kernel void a_scan_excl(device const long* incl [[buffer(0)]], device const long* f [[buffer(1)]], device long* out [[buffer(2)]],
                        constant L& p [[buffer(3)]], uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    bool reset = j == 0 || (p.flag != 0 && f[j] != 0);
    out[j] = reset ? identity(p.op) : incl[j - 1];
}

// The bits where keys differ from the first: or'd into two words (each key's bytes a sort
// passes over).
kernel void a_differs(device const long* key [[buffer(0)]], device atomic_uint* bits [[buffer(1)]], constant L& p [[buffer(2)]],
                      uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    ulong x = as_type<ulong>(key[j]) ^ as_type<ulong>(key[0]);
    if (x != 0) {
        atomic_fetch_or_explicit(&bits[0], uint(x), memory_order_relaxed);
        atomic_fetch_or_explicit(&bits[1], uint(x >> 32), memory_order_relaxed);
    }
}

// A radix pass's tile: its elements (by `perm`) sorted stably by byte `flag / 8` of their keys
// (their sign bit flipped), each one's source, digit and rank among its digit in the tile; the
// tile's count of each digit, digit-major (`hist[d * tiles + t]`).
kernel void a_radix_tile(device const long* key [[buffer(0)]], device const long* perm [[buffer(1)]], device long* ldig [[buffer(2)]],
                         device long* lsrc [[buffer(3)]], device long* lrank [[buffer(4)]], device long* hist [[buffer(5)]],
                         constant L& p [[buffer(6)]], uint j [[thread_position_in_grid]], uint lid [[thread_position_in_threadgroup]],
                         uint t [[threadgroup_position_in_grid]]) {
    threadgroup uint sd[TILE];
    threadgroup uint ss[TILE];
    threadgroup uint sc[TILE];
    threadgroup uint first[TILE];
    threadgroup uint cnt[TILE];
    const uint NO = 0xffffffffu;
    uint d, src;
    if (j < p.n) {
        src = uint(perm[j]);
        ulong k = as_type<ulong>(key[src]) ^ 0x8000000000000000ul;
        d = uint((k >> uint(p.flag)) & 255ul);
    } else {
        src = NO;
        d = 255;
    }
    sd[lid] = d;
    ss[lid] = src;
    threadgroup_barrier(mem_flags::mem_threadgroup);
    for (uint b = 0; b < 8; b++) {
        uint dd = sd[lid], s0 = ss[lid];
        uint bit = (dd >> b) & 1;
        sc[lid] = bit;
        threadgroup_barrier(mem_flags::mem_threadgroup);
        for (uint s = 1; s < TILE; s <<= 1) {
            uint add = lid >= s ? sc[lid - s] : 0;
            threadgroup_barrier(mem_flags::mem_threadgroup);
            sc[lid] += add;
            threadgroup_barrier(mem_flags::mem_threadgroup);
        }
        uint ones_before = sc[lid] - bit;
        uint zeros = TILE - sc[TILE - 1];
        uint np = bit != 0 ? zeros + ones_before : lid - ones_before;
        threadgroup_barrier(mem_flags::mem_threadgroup);
        sd[np] = dd;
        ss[np] = s0;
        threadgroup_barrier(mem_flags::mem_threadgroup);
    }
    cnt[lid] = 0;
    first[lid] = 0;
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint dd = sd[lid];
    bool valid = ss[lid] != NO;
    if (valid && (lid == 0 || sd[lid - 1] != dd)) first[dd] = lid;
    threadgroup_barrier(mem_flags::mem_threadgroup);
    if (valid && (lid == TILE - 1 || sd[lid + 1] != dd || ss[lid + 1] == NO)) cnt[dd] = lid - first[dd] + 1;
    threadgroup_barrier(mem_flags::mem_threadgroup);
    uint q = t * TILE + lid;
    ldig[q] = valid ? long(dd) : -1;
    lsrc[q] = long(ss[lid]);
    lrank[q] = valid ? long(lid - first[dd]) : 0;
    hist[lid * p.tiles + t] = long(cnt[lid]);
}

kernel void a_radix_scatter(device const long* ldig [[buffer(0)]], device const long* lsrc [[buffer(1)]], device const long* lrank [[buffer(2)]],
                            device const long* offs [[buffer(3)]], device long* out [[buffer(4)]], constant L& p [[buffer(5)]],
                            uint q [[thread_position_in_grid]]) {
    if (q >= p.tiles * TILE) return;
    long d = ldig[q];
    if (d < 0) return;
    uint t = q / TILE;
    out[offs[uint(d) * p.tiles + t] + lrank[q]] = lsrc[q];
}

kernel void a_search(device const long* s [[buffer(0)]], device const long* q [[buffer(1)]], device long* out [[buffer(2)]],
                     constant L& p [[buffer(3)]], uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    long x = q[j];
    uint lo = 0, hi = p.la;
    while (lo < hi) {
        uint mid = lo + (hi - lo) / 2;
        long e = s[mid];
        if (p.flag != 0 ? e <= x : e < x) lo = mid + 1; else hi = mid;
    }
    out[j] = long(lo);
}

kernel void a_sorted(device const long* s [[buffer(0)]], device atomic_uint* err [[buffer(1)]], constant L& p [[buffer(2)]],
                     uint j [[thread_position_in_grid]]) {
    if (j == 0 || j >= p.n) return;
    if (s[j - 1] > s[j]) fail(err, p.at, 3);
}

kernel void a_nonneg(device const long* c [[buffer(0)]], device atomic_uint* err [[buffer(1)]], constant L& p [[buffer(2)]],
                     uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    if (c[j] < 0) fail(err, p.at, 4);
}

// Where `m` is set, its index at its place (`incl`: the inclusive count of those set).
kernel void a_compact(device const long* m [[buffer(0)]], device const long* incl [[buffer(1)]], device long* out [[buffer(2)]],
                      constant L& p [[buffer(3)]], uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    if (m[j] != 0) out[incl[j] - 1] = long(j);
}

// Output `o`'s element: the first whose inclusive count passes it.
kernel void a_expand(device const long* incl [[buffer(0)]], device long* out [[buffer(1)]], constant L& p [[buffer(2)]],
                     uint o [[thread_position_in_grid]]) {
    if (o >= p.n) return;
    uint lo = 0, hi = p.la;
    long x = long(o);
    while (lo < hi) {
        uint mid = lo + (hi - lo) / 2;
        if (incl[mid] <= x) lo = mid + 1; else hi = mid;
    }
    out[o] = long(lo);
}

// A scatter's sort key: its index where it is inside `0..c`, else `c` (sorted last, written
// nowhere).
kernel void a_scatter_key(device const long* idx [[buffer(0)]], device long* out [[buffer(1)]], constant L& p [[buffer(2)]],
                          uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    long i = idx[p.la == 1 ? 0 : j];
    out[j] = (i >= 0 && i < p.c) ? i : p.c;
}

// Where an index starts its run in the sorted keys; a set scatter's second write to an index
// is an error.
kernel void a_heads(device const long* k [[buffer(0)]], device long* out [[buffer(1)]], device atomic_uint* err [[buffer(2)]],
                    constant L& p [[buffer(3)]], uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    bool head = j == 0 || k[j] != k[j - 1];
    out[j] = head ? 1 : 0;
    if (!head && p.op == 0 && k[j] < p.c) fail(err, p.at, 2);
}

// Each index's run's last: its combined values into the output, combined with the initial value.
kernel void a_scatter_write(device const long* k [[buffer(0)]], device const long* red [[buffer(1)]], device long* out [[buffer(2)]],
                            device const long* init [[buffer(3)]], constant L& p [[buffer(4)]], uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    long i = k[j];
    if (i >= p.c) return;
    if (j + 1 < p.n && k[j + 1] == i) return;
    out[i] = combine(p.op, init[0], red[j]);
}

// Whether two arrays differ anywhere.
kernel void a_differ(device const long* x [[buffer(0)]], device const long* y [[buffer(1)]], device atomic_uint* flag [[buffer(2)]],
                     constant L& p [[buffer(3)]], uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    if (x[j] != y[j]) atomic_store_explicit(flag, 1u, memory_order_relaxed);
}
