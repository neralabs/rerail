// Array programs on the device (decision 0430): each operation of `rerail_compiler::array` as
// data-parallel kernels over columns of 64-bit integers, for a batch of units at once.
// `array_program.rs` lays the program out as launches; the interpreter
// (`ArrayProgram::values_at`), run on each unit alone, is the meaning each kernel keeps.
//
// A batch's array is its units' arrays end to end. Its offsets (`o`: one a unit, and the end)
// say where each unit's begins; an element finds its unit by a binary search of them. So a
// launch serves every unit, and each keeps its own lengths: a length-1 operand broadcast in its
// unit, a scan reset at its unit's first element, a gather's or a search's indexes its own.
//
// - Elementwise operations (map, select, iota, fill, broadcast, concat, gather): a thread an
//   element.
// - Scans: a tile of 256 scanned in threadgroup memory (segment flags as pairs, a unit's first
//   element a flag), the tiles' totals scanned the same way, then added to every element before
//   its tile's first flag.
// - Sorts: a stable radix sort, a byte a pass, the unit the most significant key: each tile
//   sorted by its byte (eight one-bit splits), the tiles' counts scanned digit by digit, each
//   element written at its digit's place.
// - Searches: a bounded binary search a query, in its unit. Scatters: a stable sort by place
//   in the batch's output, a segmented scan of each place's values, and its last written.
//
// An error is each unit's first failing operation and its reason, kept by an atomic minimum.

#include <metal_stdlib>
using namespace metal;

// A launch's parameters (`Launch`).
struct L {
    uint n;
    uint nb;
    uint la;
    uint lb;
    int op;
    int flag;
    uint at;
    uint tiles;
    long c;
    long d;
};

constant uint TILE = 256;

// The unit of element `j`: the first whose end is past it.
static uint seg(device const long* o, uint nb, uint j) {
    uint lo = 0, hi = nb;
    while (lo < hi) {
        uint m = lo + (hi - lo) / 2;
        if (o[m + 1] > long(j)) hi = m; else lo = m + 1;
    }
    return lo;
}

// Element `l` of unit `u` of an operand, broadcast where its unit has one.
static uint at(device const long* o, uint u, uint l) {
    long s = o[u];
    return uint(o[u + 1] - s == 1 ? s : s + long(l));
}

static void fail(device atomic_uint* err, uint u, uint at, uint why) {
    atomic_fetch_min_explicit(&err[u], (at << 8) | why, memory_order_relaxed);
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

kernel void a_map(device const long* x [[buffer(0)]], device const long* xo [[buffer(1)]], device const long* y [[buffer(2)]],
                  device const long* yo [[buffer(3)]], device long* out [[buffer(4)]], device const long* oo [[buffer(5)]],
                  constant L& p [[buffer(6)]], uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    uint u = seg(oo, p.nb, j), l = j - uint(oo[u]);
    out[j] = map_op(p.op, x[at(xo, u, l)], y[at(yo, u, l)]);
}

kernel void a_select(device const long* m [[buffer(0)]], device const long* mo [[buffer(1)]], device const long* x [[buffer(2)]],
                     device const long* xo [[buffer(3)]], device const long* y [[buffer(4)]], device const long* yo [[buffer(5)]],
                     device long* out [[buffer(6)]], device const long* oo [[buffer(7)]], constant L& p [[buffer(8)]],
                     uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    uint u = seg(oo, p.nb, j), l = j - uint(oo[u]);
    out[j] = m[at(mo, u, l)] != 0 ? x[at(xo, u, l)] : y[at(yo, u, l)];
}

// Each element's index in its unit.
kernel void a_iota(device long* out [[buffer(0)]], device const long* oo [[buffer(1)]], constant L& p [[buffer(2)]],
                   uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    out[j] = long(j) - oo[seg(oo, p.nb, j)];
}

// Each element's unit.
kernel void a_unit(device long* out [[buffer(0)]], device const long* oo [[buffer(1)]], constant L& p [[buffer(2)]],
                   uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    out[j] = long(seg(oo, p.nb, j));
}

// `x` over `out`'s lengths: broadcast where its unit has one element, 0 past its unit's end
// (a unit already refused, whose lengths differ).
kernel void a_bcast(device const long* x [[buffer(0)]], device const long* xo [[buffer(1)]], device long* out [[buffer(2)]],
                    device const long* oo [[buffer(3)]], constant L& p [[buffer(4)]], uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    uint u = seg(oo, p.nb, j);
    long l = long(j) - oo[u], xl = xo[u + 1] - xo[u];
    out[j] = xl == 1 ? x[xo[u]] : l < xl ? x[xo[u] + l] : 0;
}

kernel void a_concat(device const long* x [[buffer(0)]], device const long* xo [[buffer(1)]], device const long* y [[buffer(2)]],
                     device const long* yo [[buffer(3)]], device long* out [[buffer(4)]], device const long* oo [[buffer(5)]],
                     constant L& p [[buffer(6)]], uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    uint u = seg(oo, p.nb, j);
    long l = long(j) - oo[u], xl = xo[u + 1] - xo[u];
    out[j] = l < xl ? x[xo[u] + l] : y[yo[u] + l - xl];
}

kernel void a_gather(device const long* s [[buffer(0)]], device const long* so [[buffer(1)]], device const long* idx [[buffer(2)]],
                     device long* out [[buffer(3)]], device const long* oo [[buffer(4)]], device atomic_uint* err [[buffer(5)]],
                     constant L& p [[buffer(6)]], uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    uint u = seg(oo, p.nb, j);
    long i = idx[j];
    if (i < 0 || i >= so[u + 1] - so[u]) {
        fail(err, u, p.at, 1);
        out[j] = 0;
    } else {
        out[j] = s[so[u] + i];
    }
}

// A tile's inclusive segmented scan: each element's value and whether a flag is at or before it
// in its tile; the tile's total and whether it holds a flag. A flag: `f`'s (`flag`), or a
// unit's first element.
kernel void a_scan_tile(device const long* x [[buffer(0)]], device const long* f [[buffer(1)]], device const long* oo [[buffer(2)]],
                        device long* out [[buffer(3)]], device long* fl [[buffer(4)]], device long* agg [[buffer(5)]],
                        device long* aggf [[buffer(6)]], constant L& p [[buffer(7)]], uint j [[thread_position_in_grid]],
                        uint lid [[thread_position_in_threadgroup]], uint t [[threadgroup_position_in_grid]]) {
    threadgroup long sv[TILE];
    threadgroup uchar sf[TILE];
    long v = identity(p.op);
    uchar g = 0;
    if (j < p.n) {
        v = x[j];
        g = ((p.flag != 0 && f[j] != 0) || long(j) == oo[seg(oo, p.nb, j)]) ? 1 : 0;
    }
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

// An exclusive scan from the inclusive: the identity at a flag and at a unit's first element.
kernel void a_scan_excl(device const long* incl [[buffer(0)]], device const long* f [[buffer(1)]], device const long* oo [[buffer(2)]],
                        device long* out [[buffer(3)]], constant L& p [[buffer(4)]], uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    bool reset = long(j) == oo[seg(oo, p.nb, j)] || (p.flag != 0 && f[j] != 0);
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

// A sorted permutation's sources as indexes in their units (each unit's elements sorted among
// its own: the unit the most significant key).
kernel void a_localize(device long* perm [[buffer(0)]], device const long* oo [[buffer(1)]], constant L& p [[buffer(2)]],
                       uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    perm[j] -= oo[seg(oo, p.nb, j)];
}

kernel void a_search(device const long* s [[buffer(0)]], device const long* so [[buffer(1)]], device const long* q [[buffer(2)]],
                     device long* out [[buffer(3)]], device const long* oo [[buffer(4)]], constant L& p [[buffer(5)]],
                     uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    uint u = seg(oo, p.nb, j);
    long x = q[j];
    long base = so[u];
    uint lo = 0, hi = uint(so[u + 1] - base);
    while (lo < hi) {
        uint mid = lo + (hi - lo) / 2;
        long e = s[base + mid];
        if (p.flag != 0 ? e <= x : e < x) lo = mid + 1; else hi = mid;
    }
    out[j] = long(lo);
}

kernel void a_sorted(device const long* s [[buffer(0)]], device const long* so [[buffer(1)]], device atomic_uint* err [[buffer(2)]],
                     constant L& p [[buffer(3)]], uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    uint u = seg(so, p.nb, j);
    if (long(j) != so[u] && s[j - 1] > s[j]) fail(err, u, p.at, 3);
}

// A count refused (`a_nonneg`) where it is negative; the counts an expand reads: 0 where
// negative and in a unit already refused (on the device, or by the host: `dead`).
kernel void a_counts(device const long* c [[buffer(0)]], device const long* co [[buffer(1)]], device atomic_uint* err [[buffer(2)]],
                     device const uint* dead [[buffer(3)]], device long* out [[buffer(4)]], constant L& p [[buffer(5)]],
                     uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    uint u = seg(co, p.nb, j);
    long k = c[j];
    if (k < 0) fail(err, u, p.at, 4);
    bool gone = dead[u] != 0 || atomic_load_explicit(&err[u], memory_order_relaxed) != 0xffffffffu;
    out[j] = (k < 0 || gone) ? 0 : k;
}

// Where `m` is set, its index in its unit at its place (`incl`: the batch's inclusive count of
// those set).
kernel void a_compact(device const long* m [[buffer(0)]], device const long* mo [[buffer(1)]], device const long* incl [[buffer(2)]],
                      device long* out [[buffer(3)]], constant L& p [[buffer(4)]], uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    if (m[j] != 0) out[incl[j] - 1] = long(j) - mo[seg(mo, p.nb, j)];
}

// Output `o`'s element: the first whose inclusive count passes it, as an index in its unit.
kernel void a_expand(device const long* incl [[buffer(0)]], device const long* co [[buffer(1)]], device long* out [[buffer(2)]],
                     constant L& p [[buffer(3)]], uint o [[thread_position_in_grid]]) {
    if (o >= p.n) return;
    uint lo = 0, hi = p.la;
    long x = long(o);
    while (lo < hi) {
        uint mid = lo + (hi - lo) / 2;
        if (incl[mid] <= x) lo = mid + 1; else hi = mid;
    }
    out[o] = long(lo) - co[seg(co, p.nb, lo)];
}

// A scatter's sort key: its place in the batch's output where its index is inside its unit's
// `0..n`, else `c` (the output's length: sorted last, written nowhere).
kernel void a_scatter_key(device const long* idx [[buffer(0)]], device const long* wo [[buffer(1)]], device const long* oo [[buffer(2)]],
                          device long* out [[buffer(3)]], constant L& p [[buffer(4)]], uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    uint u = seg(wo, p.nb, j);
    long i = idx[j];
    out[j] = (i >= 0 && i < oo[u + 1] - oo[u]) ? oo[u] + i : p.c;
}

// Where a place starts its run in the sorted keys; a set scatter's second write to a place is
// its unit's error.
kernel void a_heads(device const long* k [[buffer(0)]], device long* out [[buffer(1)]], device const long* oo [[buffer(2)]],
                    device atomic_uint* err [[buffer(3)]], constant L& p [[buffer(4)]], uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    bool head = j == 0 || k[j] != k[j - 1];
    out[j] = head ? 1 : 0;
    if (!head && p.op == 0 && k[j] < p.c) fail(err, seg(oo, p.nb, uint(k[j])), p.at, 2);
}

// Each output element its unit's initial value.
kernel void a_fill(device long* out [[buffer(0)]], device const long* oo [[buffer(1)]], device const long* v [[buffer(2)]],
                   device const long* vo [[buffer(3)]], constant L& p [[buffer(4)]], uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    out[j] = v[vo[seg(oo, p.nb, j)]];
}

// Each place's run's last: its combined values into the output, combined with its unit's
// initial value.
kernel void a_scatter_write(device const long* k [[buffer(0)]], device const long* red [[buffer(1)]], device long* out [[buffer(2)]],
                            device const long* oo [[buffer(3)]], device const long* init [[buffer(4)]], device const long* io [[buffer(5)]],
                            constant L& p [[buffer(6)]], uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    long i = k[j];
    if (i >= p.c) return;
    if (j + 1 < p.n && k[j + 1] == i) return;
    out[i] = combine(p.op, init[io[seg(oo, p.nb, uint(i))]], red[j]);
}

// Which units' arrays differ (where their lengths are the same; the host compares lengths).
kernel void a_differ(device const long* x [[buffer(0)]], device const long* xo [[buffer(1)]], device const long* y [[buffer(2)]],
                     device const long* yo [[buffer(3)]], device uint* flags [[buffer(4)]], constant L& p [[buffer(5)]],
                     uint j [[thread_position_in_grid]]) {
    if (j >= p.n) return;
    uint u = seg(xo, p.nb, j);
    long l = long(j) - xo[u];
    if (yo[u + 1] - yo[u] != xo[u + 1] - xo[u]) return;
    if (x[j] != y[yo[u] + l]) flags[u] = 1;
}
