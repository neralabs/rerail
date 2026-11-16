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
//! t39 step 23 adds outermost by a prefix max (no climb) and nearest by a bounded climb, on XF, MF and format 16.
//! Format 16's kernels (t39 step 22, `f16_*`) read the tape as stored, through `inplace_gpu`'s `F16C`, with
//! the set compiled as the CPU reader's `NumSet`. Every answer is checked against the tree.

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

// ---- has from counts: has(a) is "an S-node ranks after a and before a's close", so with S(r), the S-nodes
// ranked below r (a scan of the groups' mask counts), it is S(close rank) − S(a + 1) > 0. No climb. ----
#define CARGS device const uchar* buf [[buffer(0)]], constant uint* P [[buffer(1)]], device const uchar* tabs [[buffer(2)]], \
    device const uchar* set [[buffer(3)]], device ulong* masks [[buffer(4)]], device uint* out [[buffer(5)]], \
    constant uint& mode [[buffer(6)]], device atomic_uint* bad [[buffer(7)]], device uint* cnt [[buffer(14)]], \
    device uint* cex [[buffer(15)]], device ulong2* span [[buffer(16)]], constant uint* lv [[buffer(12)]], \
    constant uint& level [[buffer(13)]], uint g [[thread_position_in_grid]]

kernel void c_leaf(CARGS) { if (g < lv[2]) cnt[g] = popcount(masks[g]); }
kernel void c_up(CARGS) {
    uint off = lv[1 + 2 * level], n = lv[2 + 2 * level], poff = lv[3 + 2 * level], pn = lv[4 + 2 * level];
    if (g >= pn) return;
    uint a = 0;
    for (uint c = g * 64; c < min(n, g * 64 + 64); c++) a += cnt[off + c];
    cnt[poff + g] = a;
}
kernel void c_down(CARGS) {
    uint off = lv[1 + 2 * level], n = lv[2 + 2 * level], poff = lv[3 + 2 * level], pn = lv[4 + 2 * level];
    if (g >= pn) return;
    uint a = level + 2 == lv[0] ? 0 : cex[poff + g];
    for (uint c = g * 64; c < min(n, g * 64 + 64); c++) { cex[off + c] = a; a += cnt[off + c]; }
}
// S-nodes ranked below r.
inline uint s_below(device const uint* cex, device const uint* cnt, device const ulong* masks, uint groups, uint r) {
    uint q = r >> 6;
    if (q >= groups) return cex[groups - 1] + cnt[groups - 1];
    return cex[q] + popcount(masks[q] & ((1ul << (r & 63)) - 1ul));
}

kernel void xf_hasc(CARGS) {
    Rd r = { buf, P, 0 };
    XFC x = rd_xf(r);
    uint k0 = g * 64;
    if (k0 >= x.n) return;
    uint G = lv[2];
    for (uint k = k0; k < min(x.n, k0 + 64); k++) {
        out[k] = s_below(cex, cnt, masks, G, x.end(k)) > s_below(cex, cnt, masks, G, k + 1) ? 1u : 0u;
    }
}

// BE: a node that closes in its group's walk has its close rank there (the opens so far); the few still open
// at the walk's end find their close.
kernel void b_hasc(CARGS) {
    Rd r = { buf, P, 0 };
    BC b = rd_b(r);
    Tab tb = { tabs };
    uint k0 = g * 64;
    if (k0 >= b.n) return;
    uint G = lv[2];
    uint q = b_select1(b, k0, tb);
    uint st[64], sq[64];
    int sp = 0;
    uint hi = min(64u, b.n - k0), c = 0;
    while (q < b.nbits) {
        ulong word = b.bits[q >> 6] >> (q & 63);
        uint left = 64 - (q & 63);
        bool done = false;
        for (uint i = 0; i < left && q < b.nbits; i++, q++) {
            if ((word >> i) & 1) {
                if (c == hi) { done = true; break; }
                st[sp] = k0 + c; sq[sp] = q; sp++;
                c++;
            } else if (sp > 0) {
                sp--;
                out[st[sp]] = s_below(cex, cnt, masks, G, k0 + c) > s_below(cex, cnt, masks, G, st[sp] + 1) ? 1u : 0u;
            }
        }
        if (done) break;
    }
    for (int i = 0; i < sp; i++) {
        uint cr = b.rank1(b.findclose(sq[i], tb));
        out[st[i]] = s_below(cex, cnt, masks, G, cr) > s_below(cex, cnt, masks, G, st[i] + 1) ? 1u : 0u;
    }
}

// ---- spans: (start, end) of every node ----
kernel void xfl_span(CARGS) {
    Rd r = { buf, P, 0 };
    XFC x = rd_xf(r); PFor st = rd_pfor(r); PFor ln = rd_pfor(r);
    uint k0 = g * 64;
    if (k0 >= x.n) return;
    for (uint k = k0; k < min(x.n, k0 + 64); k++) { ulong s0 = st.get(k); span[k] = ulong2(s0, s0 + ln.get(k)); }
}
// MF: the starts first; then each end is the answer's start just past the subtree, less the gap.
kernel void xfg_start(CARGS) {
    Rd r = { buf, P, 0 };
    XFC x = rd_xf(r); Pred st = rd_pred(r);
    uint k0 = g * 64;
    if (k0 >= x.n) return;
    for (uint k = k0; k < min(x.n, k0 + 64); k++) span[k].x = st.get(k);
}
kernel void xfg_end(CARGS) {
    Rd r = { buf, P, 0 };
    XFC x = rd_xf(r); Pred st = rd_pred(r); PFor gap = rd_pfor(r);
    uint n = r.val(); ulong src = ulong(r.val()) | (ulong(r.val()) << 32);
    uint k0 = g * 64;
    if (k0 >= x.n) return;
    for (uint k = k0; k < min(n, k0 + 64); k++) {
        uint nx = x.end(k);
        ulong s1 = nx < n ? span[nx].x : src;
        ulong z = gap.get(k);
        span[k].y = ulong(long(s1) - (long(z >> 1) ^ -long(z & 1)));
    }
}
// BE: ends in postorder; a close's postorder number is the closes before it (its position less the opens).
kernel void b_span(CARGS) {
    Rd r = { buf, P, 0 };
    BC b = rd_b(r); Ef st = rd_ef(r); Ef en = rd_ef(r);
    Tab tb = { tabs };
    uint k0 = g * 64;
    if (k0 >= b.n) return;
    uint q = b_select1(b, k0, tb);
    uint sk[64], sq[64];
    int sp = 0;
    uint hi = min(64u, b.n - k0), c = 0;
    for (uint k = k0; k < k0 + hi; k++) span[k].x = st.get(k, tb);
    while (q < b.nbits) {
        ulong word = b.bits[q >> 6] >> (q & 63);
        uint left = 64 - (q & 63);
        bool done = false;
        for (uint i = 0; i < left && q < b.nbits; i++, q++) {
            if ((word >> i) & 1) {
                if (c == hi) { done = true; break; }
                sk[sp] = k0 + c; sq[sp] = q; sp++;
                c++;
            } else if (sp > 0) {
                sp--;
                span[sk[sp]].y = en.get(q - (k0 + c), tb);
            }
        }
        if (done) break;
    }
    for (int i = 0; i < sp; i++) {
        uint cp = b.findclose(sq[i], tb);
        span[sk[i]].y = en.get(cp - b.rank1(cp), tb);
    }
}
// ---- format 16 (t39 step 22): the same primitives on the tape as stored (`inplace_gpu`'s `F16C`). The set is
// compiled once for the language, as the CPU reader's (`f16::NumSet`): 256 byte entries, the prefix's entry
// saying whether the set holds any code past the cutoff, then the set by number. ----
kernel void f16_match(BARGS) {
    Rd r = { buf, P, 0 };
    F16C x = rd_f16(r);
    uint k0 = g * 64;
    if (k0 >= x.n) return;
    // The group's 64 code bytes as 8 words (the bytes are padded to whole groups). A byte past the cutoff is
    // decoded only when the set holds such a code (the prefixes' entries are that one flag); the group's
    // nodes past it are its block's next entries in order, so one search finds the first and the rest follow.
    device const ulong* w = (device const ulong*)(x.code.b + k0);
    ulong m = 0;
    uint idx = 0, vb = 0;
    bool found = false;
    for (uint q = 0; q < 8; q++) {
        ulong word = w[q];
        for (uint j = 0; j < 8; j++) {
            uint v = uint(word >> (8 * j)) & 255u, k = q * 8 + j;
            uint b = set[v];
            if (v >= x.code.cut && b != 0) {
                if (found) { idx++; } else { idx = x.code.parts.find(k0 + k, vb); found = true; }
                b = set[256 + x.code.cut + (v - x.code.cut) * x.code.per + uint(x.code.parts.value(vb, idx))];
            }
            m |= ulong(b) << k;
        }
    }
    masks[g] = m & mask(min(64u, x.n - k0));
}

// mode 0: nearest, 1: outermost.
kernel void f16_nearest(BARGS) {
    Rd r = { buf, P, 0 };
    F16C x = rd_f16(r);
    uint k0 = g * 64;
    if (k0 >= x.n) return;
    uint st[STACK], se[STACK];
    int sp = 0;
    ulong p1 = x.parent1(k0);
    while (p1 != 0) {
        uint p = uint(p1 - 1);
        if (set[256 + x.code.get(p)]) {
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

kernel void f16_has(BARGS) {
    Rd r = { buf, P, 0 };
    F16C x = rd_f16(r);
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

kernel void f16_hasc(CARGS) {
    Rd r = { buf, P, 0 };
    F16C x = rd_f16(r);
    uint k0 = g * 64;
    if (k0 >= x.n) return;
    uint G = lv[2];
    for (uint k = k0; k < min(x.n, k0 + 64); k++) {
        out[k] = s_below(cex, cnt, masks, G, x.end(k)) > s_below(cex, cnt, masks, G, k + 1) ? 1u : 0u;
    }
}

// A group of 64 starts on a checkpoint (64 is a multiple of K): a running sum of gaps, a checkpoint every 8.
kernel void f16_span(CARGS) {
    Rd r = { buf, P, 0 };
    F16C x = rd_f16(r);
    uint k0 = g * 64;
    if (k0 >= x.n) return;
    ulong s0 = 0;
    for (uint k = k0; k < min(x.n, k0 + 64); k++) {
        s0 = (k & 7) == 0 ? x.ck.get(k >> 3) : s0 + x.gap.get(k);
        span[k] = ulong2(s0, s0 + x.len.get(k));
    }
}
// ---- t39 step 23: outermost with no climb, nearest with a bounded one. Among the S-nodes before k, the one whose
// subtree ends last (the earliest on a tie) is k's outermost S-ancestor when that end is past k, and k has none
// otherwise: a prefix max over the groups (key = end << 32 | ~index), up and down the same levels as the
// counts. Nearest then climbs from a group's first node only when it has an S-ancestor, and stops at the first
// S-ancestor that encloses the whole group, or at the outermost. `mode`: the set's offset (format 16's set by
// number follows its 256 byte entries). ----
#define OARGS device const uchar* buf [[buffer(0)]], constant uint* P [[buffer(1)]], device const uchar* set [[buffer(3)]], \
    device ulong* masks [[buffer(4)]], device uint* out [[buffer(5)]], constant uint& mode [[buffer(6)]], \
    device atomic_uint* bad [[buffer(7)]], device ulong* okey [[buffer(8)]], device ulong* oex [[buffer(9)]], \
    constant uint* lv [[buffer(12)]], constant uint& level [[buffer(13)]], uint g [[thread_position_in_grid]]

inline ulong okey_of(uint k, uint e) { return (ulong(e) << 32) | ulong(0xffffffffu - k); }

template <typename T> inline void okey_t(thread T& x, device const ulong* masks, device ulong* okey, uint g) {
    uint k0 = g * 64;
    if (k0 >= x.n) return;
    ulong m = masks[g], run = 0;
    while (m != 0) { uint k = k0 + ctz(m); m &= m - 1; run = max(run, okey_of(k, x.end(k))); }
    okey[g] = run;
}
kernel void o_up(OARGS) {
    uint off = lv[1 + 2 * level], n = lv[2 + 2 * level], poff = lv[3 + 2 * level], pn = lv[4 + 2 * level];
    if (g >= pn) return;
    ulong a = 0;
    for (uint c = g * 64; c < min(n, g * 64 + 64); c++) a = max(a, okey[off + c]);
    okey[poff + g] = a;
}
kernel void o_down(OARGS) {
    uint off = lv[1 + 2 * level], n = lv[2 + 2 * level], poff = lv[3 + 2 * level], pn = lv[4 + 2 * level];
    if (g >= pn) return;
    ulong a = level + 2 == lv[0] ? 0 : oex[poff + g];
    for (uint c = g * 64; c < min(n, g * 64 + 64); c++) { oex[off + c] = a; a = max(a, okey[off + c]); }
}
template <typename T> inline void outer2_t(thread T& x, device const ulong* masks, device const ulong* oex, device uint* out, uint g) {
    uint k0 = g * 64;
    if (k0 >= x.n) return;
    ulong run = oex[g], m = masks[g];
    for (uint k = k0; k < min(x.n, k0 + 64); k++) {
        out[k] = uint(run >> 32) > k ? 0xffffffffu - uint(run) : NONE;
        if ((m >> (k - k0)) & 1) run = max(run, okey_of(k, x.end(k)));
    }
}
template <typename T> inline void near2_t(thread T& x, device const uchar* set, uint so, device const ulong* masks,
        device const ulong* oex, device uint* out, device atomic_uint* bad, uint g) {
    uint k0 = g * 64;
    if (k0 >= x.n) return;
    uint ge = min(x.n, k0 + 64);
    uint st[STACK], se[STACK];
    int sp = 0;
    ulong run = oex[g];
    if (uint(run >> 32) > k0) {
        uint top = 0xffffffffu - uint(run);
        ulong p1 = x.parent1(k0);
        while (p1 != 0) {
            uint p = uint(p1 - 1);
            if (set[so + x.code.get(p)]) {
                if (sp == STACK) { atomic_fetch_add_explicit(bad, 1u, memory_order_relaxed); return; }
                uint e = x.end(p);
                st[sp] = p; se[sp] = e; sp++;
                if (p == top || e >= ge) break;
            }
            p1 = x.parent1(p);
        }
    }
    for (int i = 0; i < sp / 2; i++) {
        uint a = st[i]; st[i] = st[sp - 1 - i]; st[sp - 1 - i] = a;
        uint b = se[i]; se[i] = se[sp - 1 - i]; se[sp - 1 - i] = b;
    }
    ulong m = masks[g];
    for (uint k = k0; k < ge; k++) {
        while (sp > 0 && se[sp - 1] <= k) sp--;
        out[k] = sp == 0 ? NONE : st[sp - 1];
        if ((m >> (k - k0)) & 1) {
            if (sp == STACK) { atomic_fetch_add_explicit(bad, 1u, memory_order_relaxed); return; }
            st[sp] = k; se[sp] = x.end(k); sp++;
        }
    }
}
// Nearest with no parent read: going back over the S-nodes before k0 (the masks, a word at a time), the first
// whose subtree ends past k0 is its nearest S-ancestor, and each later one that does is the next ancestor out;
// the S-nodes that closed before k0 cost one end each. It stops at an S-ancestor enclosing the whole group, or
// at the outermost; past SCAN words back it falls back to the bounded climb.
#define SCAN 64
template <typename T> inline void near3_t(thread T& x, device const uchar* set, uint so, device const ulong* masks,
        device const ulong* oex, device uint* out, device atomic_uint* bad, uint g) {
    uint k0 = g * 64;
    if (k0 >= x.n) return;
    uint ge = min(x.n, k0 + 64);
    uint st[STACK], se[STACK];
    int sp = 0;
    ulong run = oex[g];
    if (uint(run >> 32) > k0) {
        uint top = 0xffffffffu - uint(run);
        bool done = false;
        for (uint w = g, steps = 0; w-- > 0 && steps < SCAN && !done; steps++) {
            ulong m = masks[w];
            while (m != 0) {
                uint s = w * 64 + 63 - clz(m);
                m &= ~(1ul << (s - w * 64));
                uint e = x.end(s);
                if (e > k0) {
                    if (sp == STACK) { atomic_fetch_add_explicit(bad, 1u, memory_order_relaxed); return; }
                    st[sp] = s; se[sp] = e; sp++;
                    if (s == top || e >= ge) { done = true; break; }
                }
            }
        }
        if (!done) {
            // Too far back: the bounded climb from k0 instead.
            sp = 0;
            ulong p1 = x.parent1(k0);
            while (p1 != 0) {
                uint p = uint(p1 - 1);
                if (set[so + x.code.get(p)]) {
                    if (sp == STACK) { atomic_fetch_add_explicit(bad, 1u, memory_order_relaxed); return; }
                    uint e = x.end(p);
                    st[sp] = p; se[sp] = e; sp++;
                    if (p == top || e >= ge) break;
                }
                p1 = x.parent1(p);
            }
        }
    }
    for (int i = 0; i < sp / 2; i++) {
        uint a = st[i]; st[i] = st[sp - 1 - i]; st[sp - 1 - i] = a;
        uint b = se[i]; se[i] = se[sp - 1 - i]; se[sp - 1 - i] = b;
    }
    ulong m = masks[g];
    for (uint k = k0; k < ge; k++) {
        while (sp > 0 && se[sp - 1] <= k) sp--;
        out[k] = sp == 0 ? NONE : st[sp - 1];
        if ((m >> (k - k0)) & 1) {
            if (sp == STACK) { atomic_fetch_add_explicit(bad, 1u, memory_order_relaxed); return; }
            st[sp] = k; se[sp] = x.end(k); sp++;
        }
    }
}
kernel void xf_near3(OARGS) { Rd r = { buf, P, 0 }; XFC x = rd_xf(r); near3_t(x, set, mode, masks, oex, out, bad, g); }
kernel void f16_near3(OARGS) { Rd r = { buf, P, 0 }; F16C x = rd_f16(r); near3_t(x, set, mode, masks, oex, out, bad, g); }
kernel void xf_okey(OARGS) { Rd r = { buf, P, 0 }; XFC x = rd_xf(r); okey_t(x, masks, okey, g); }
kernel void xf_outer2(OARGS) { Rd r = { buf, P, 0 }; XFC x = rd_xf(r); outer2_t(x, masks, oex, out, g); }
kernel void xf_near2(OARGS) { Rd r = { buf, P, 0 }; XFC x = rd_xf(r); near2_t(x, set, mode, masks, oex, out, bad, g); }
kernel void f16_okey(OARGS) { Rd r = { buf, P, 0 }; F16C x = rd_f16(r); okey_t(x, masks, okey, g); }
kernel void f16_outer2(OARGS) { Rd r = { buf, P, 0 }; F16C x = rd_f16(r); outer2_t(x, masks, oex, out, g); }
kernel void f16_near2(OARGS) { Rd r = { buf, P, 0 }; F16C x = rd_f16(r); near2_t(x, set, mode, masks, oex, out, bad, g); }
"#;


pub struct Row {
    /// GPU ns a node, best of 8 warm dispatches: match, nearest, outermost, has (each with its match).
    pub ns: [f64; 4],
    /// BE: nearest with the carry found by climbing (the first form), against the scan in `ns`.
    pub no_carry: Option<f64>,
    /// has from counts (the mask counts scanned, then a formula a node), and every node's span.
    pub has_counts: f64,
    /// t39 step 23: nearest with the bounded climb and outermost from the prefix max (XF, MF, format 16).
    pub scan: Option<(f64, f64)>,
    /// Nearest by the S-nodes back from each group (no parent read), with the prefix max.
    pub back: Option<f64>,
    pub span: f64,
    pub wrong: usize,
}

/// One candidate, one set: the kernels `<pre>_match`, `<pre>_nearest`, `<pre>_has` over its image.
pub fn run(pre: &str, span_kernels: &[&str], image: &GpuImage, n: usize, set: &[bool], truth: (&[u32], &[u32], &[bool]),
    truth_span: &[(u64, u64)]) -> Result<Row, String> {
    let device = Device::system_default().ok_or("no Metal device")?;
    let src = format!("{}{}", crate::inplace_gpu::SOURCE, KERNELS);
    let lib = device.new_library_with_source(&src, &metal::CompileOptions::new()).map_err(|e| e.to_string())?;
    let lib_pipe = |name: &str| -> Result<metal::ComputePipelineState, String> {
        let f = lib.get_function(name, None).map_err(|e| e.to_string())?;
        device.new_compute_pipeline_state_with_function(&f).map_err(|e| e.to_string())
    };
    let pipe = |name: &str| -> Result<metal::ComputePipelineState, String> {
        let pref = format!("{pre}_{name}");
        lib_pipe(if name.starts_with("c_") { name } else { &pref })
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
    let (bcnt, bcex) = (device.new_buffer((off * 4) as u64, opts), device.new_buffer((off * 4) as u64, opts));
    let bspan = device.new_buffer((n * 16).max(16) as u64, opts);
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
                for (i, b) in [&bcnt, &bcex, &bspan].into_iter().enumerate() {
                    enc.set_buffer(14 + i as u64, Some(b), 0);
                }
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
    // has from counts: the mask counts scanned up and down the levels, then a formula a node.
    let (pcl, pcu, pcd, phc) = (pipe("c_leaf")?, pipe("c_up")?, pipe("c_down")?, lib_pipe(&format!("{pre}_hasc"))?);
    let mut st: Vec<Stage> = vec![(&pm, 0, 0, g), (&pcl, 0, 0, g)];
    for l in 0..levels - 1 {
        st.push((&pcu, 0, l as u32, lv[4 + 2 * l] as usize));
    }
    for l in (0..levels - 1).rev() {
        st.push((&pcd, 0, l as u32, lv[4 + 2 * l] as usize));
    }
    st.push((&phc, 0, 0, g));
    let t_hasc = time(&st, false);
    wrong += got().iter().zip(truth.2).filter(|&(a, &b)| (*a == 1) != b).count();
    // Outermost from the prefix max, and nearest with the climb it bounds.
    let mut t_back = None;
    let scan = if pre == "b" {
        None
    } else {
        let so = if pre == "f16" { 256 } else { 0 };
        let (pok, pou, pod, pn2, po2) = (pipe("okey")?, lib_pipe("o_up")?, lib_pipe("o_down")?, pipe("near2")?, pipe("outer2")?);
        let mut st: Vec<Stage> = vec![(&pm, so, 0, g), (&pok, so, 0, g)];
        for l in 0..levels - 1 {
            st.push((&pou, so, l as u32, lv[4 + 2 * l] as usize));
        }
        for l in (0..levels - 1).rev() {
            st.push((&pod, so, l as u32, lv[4 + 2 * l] as usize));
        }
        let mut sn = st.clone();
        sn.push((&pn2, so, 0, g));
        let t_n2 = time(&sn, false);
        wrong += got().iter().zip(truth.0).filter(|(a, b)| a != b).count();
        let pn3 = pipe("near3")?;
        let mut sb = st.clone();
        sb.push((&pn3, so, 0, g));
        t_back = Some(time(&sb, false));
        wrong += got().iter().zip(truth.0).filter(|(a, b)| a != b).count();
        st.push((&po2, so, 0, g));
        let t_o2 = time(&st, false);
        wrong += got().iter().zip(truth.1).filter(|(a, b)| a != b).count();
        Some((t_n2, t_o2))
    };
    // Spans.
    let sp: Vec<metal::ComputePipelineState> = span_kernels.iter().map(|k| lib_pipe(k)).collect::<Result<_, _>>()?;
    let st: Vec<Stage> = sp.iter().map(|p| (p, 0, 0, g)).collect();
    let t_span = time(&st, false);
    // SAFETY: shared storage of n (u64, u64), and the GPU is done with it.
    let spans = unsafe { std::slice::from_raw_parts(bspan.contents().cast::<(u64, u64)>(), n) };
    wrong += spans.iter().zip(truth_span).filter(|(a, b)| a != b).count();
    // SAFETY: as above.
    wrong += unsafe { *bbad.contents().cast::<u32>() } as usize;
    Ok(Row { ns: [t_match, t_near, t_outer, t_has], no_carry, has_counts: t_hasc, scan, back: t_back, span: t_span, wrong })
}

/// XF, MF, BE and format 16 on one forest, for one set: (name, GPU row, CPU ns a node for nearest, outermost, has from
/// the S-nodes, best of 3).
/// `table`: the language's code table from the grammar, for format 16's numbering (`f16::grammar_codes`).
pub fn all(tree: &crate::tree::Tree, codes: &crate::inplace::Codes, set: &[bool], table: usize) -> Result<Vec<(&'static str, Row, [f64; 3])>, String> {
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
    let ts: Vec<(u64, u64)> = tree.nodes.iter().map(|x| (x.start, x.end)).collect();
    let mut rows = Vec::new();
    let xf = XF::<ForLen>::new(tree, codes);
    let (c, w) = cpu(&xf, set, n, t);
    let mut row = run("xf", &["xfl_span"], &xf.gpu_image(), n, set, t, &ts)?;
    row.wrong += w;
    rows.push(("XF", row, c));
    drop(xf);
    let mf = XF::<PredGap>::new(tree, codes);
    let (c, w) = cpu(&mf, set, n, t);
    let mut row = run("xf", &["xfg_start", "xfg_end"], &mf.gpu_image(), n, set, t, &ts)?;
    row.wrong += w;
    rows.push(("MF", row, c));
    drop(mf);
    let be = B::<EfEf>::new(tree, codes);
    let (c, w) = cpu(&be, set, n, t);
    let mut row = run("b", &["b_span"], &be.gpu_image(), n, set, t, &ts)?;
    row.wrong += w;
    rows.push(("BE", row, c));
    drop(be);
    // Format 16, its choices learned on this forest; the kernels test the set by number.
    let cols = crate::inplace::Cols::of_tree(tree, codes);
    let lang = crate::f16::F16Lang::learn(codes, table, std::iter::once(&cols)).marked(0);
    let f = crate::f16::F16::new(&cols, &lang);
    drop(cols);
    let by_num: Vec<bool> = lang.by_num.iter().map(|&k| set.get(k as usize).copied().unwrap_or(false)).collect();
    let cut = (lang.cut as usize).min(by_num.len());
    let mut f16_set: Vec<bool> = (0..256).map(|v| v < cut && by_num[v]).collect();
    if cut < 256 {
        f16_set[cut..].fill(by_num[cut..].iter().any(|&b| b));
    }
    f16_set.extend(&by_num);
    let (c, w) = cpu(&f, set, n, t);
    let mut row = run("f16", &["f16_span"], &f.gpu_image(), n, &f16_set, t, &ts)?;
    row.wrong += w;
    rows.push(("F16", row, c));
    Ok(rows)
}
