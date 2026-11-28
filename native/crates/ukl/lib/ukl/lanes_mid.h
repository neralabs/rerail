// ukl/lanes_mid.h: a range's summary from the middle on the device (decision 0726), as tier 2's `Lane`, `Refusal` and
// `fold` are in its Rust: the lanes a range's walk leaves from each place it can start at, a thread a range.
//
// A lane is a record of words in a pool, at the offset its summary keeps:
//
//   0 `st0`, 1 `kc`'s length, 2 `p`'s, 3-6 `exit` (`i`, `e`, `depth`, `st`), 7 `nodes`, 8 `end` (0 none, 1 the file's
//   end, 2 its refusal), 9 the refusal's byte, 10 `fkc`, 11 `touch`'s length, 12 `pf`'s, 13-14 `last_start` (low,
//   high), 15 whether it has one, 16 `refuse`'s length; then `kc`, `p` (a word each), `touch` (two words each:
//   children, last or `UKL_LANE_NONE`), `pf` (`UKL_LANE_FRAME` words each: node, children, last, start's low and high
//   words) and `refuse` (`UKL_LANE_REFUSAL` words each: level, places, byte, nodes, last start or `UKL_LANE_NONE`).
//
// A pool is taken by an atomic bump of its first word; a range whose lanes pass its room says so, and the run is made
// again with more (the engine's `room`).
#pragma once
#include <ukl/backend.h>

#define UKL_LANE_NONE 4294967295u
#define UKL_LANE_HEAD 17u
#define UKL_LANE_FRAME 5u
#define UKL_LANE_REFUSAL 5u

#define UKL_LANE_ST0 0u
#define UKL_LANE_NKC 1u
#define UKL_LANE_NP 2u
#define UKL_LANE_EXIT 3u
#define UKL_LANE_NODES 7u
#define UKL_LANE_END 8u
#define UKL_LANE_ERR 9u
#define UKL_LANE_FKC 10u
#define UKL_LANE_NTOUCH 11u
#define UKL_LANE_NPF 12u
#define UKL_LANE_LAST 13u
#define UKL_LANE_HAS_LAST 15u
#define UKL_LANE_NREF 16u

typedef UKL_GLOBAL uint32_t* UklLanePool;

UKL_INLINE uint32_t ukl_lane_kc(UklLanePool m, uint32_t l) { return l + UKL_LANE_HEAD; }
UKL_INLINE uint32_t ukl_lane_p(UklLanePool m, uint32_t l) { return ukl_lane_kc(m, l) + m[l + UKL_LANE_NKC]; }
UKL_INLINE uint32_t ukl_lane_touch(UklLanePool m, uint32_t l) { return ukl_lane_p(m, l) + m[l + UKL_LANE_NP]; }
UKL_INLINE uint32_t ukl_lane_pf(UklLanePool m, uint32_t l) { return ukl_lane_touch(m, l) + 2u * m[l + UKL_LANE_NTOUCH]; }
UKL_INLINE uint32_t ukl_lane_ref(UklLanePool m, uint32_t l) { return ukl_lane_pf(m, l) + UKL_LANE_FRAME * m[l + UKL_LANE_NPF]; }
UKL_INLINE uint32_t ukl_lane_size(UklLanePool m, uint32_t l) { return ukl_lane_ref(m, l) + UKL_LANE_REFUSAL * m[l + UKL_LANE_NREF] - l; }

// `n` words of the pool (its first word the next free one, its second its size), or `UKL_LANE_NONE` past its size.
UKL_INLINE uint32_t ukl_lane_take(UklLanePool m, uint32_t n)
{
    uint32_t at = ukl_atomic_add(m, n);
    return at + n > m[1] ? UKL_LANE_NONE : at;
}

// Whether refusal `a` comes before refusal `b` (`fold`'s order: level, places, byte, nodes, last start).
UKL_INLINE int ukl_lane_ref_cmp(UklLanePool m, uint32_t a, uint32_t b)
{
    for (uint32_t k = 0u; k < UKL_LANE_REFUSAL; k++)
        if (m[a + k] != m[b + k])
            return m[a + k] < m[b + k] ? -1 : 1;
    return 0;
}

// The lanes at `in[0..n]` folded (`fold`): sorted by their closed levels, most first (a stable sort); a lane refused
// where its last level returned to another place than a kept lane's, the levels below as that lane's, made that lane's
// refusal, with its own. The kept lanes are written anew, their refusals sorted and each once; their offsets in
// `out`, their count returned, or `UKL_LANE_NONE` where the pool or `most` is passed. `abs` is `n` words of scratch.
UKL_INLINE uint32_t ukl_lane_fold(UklLanePool m, UKL_GLOBAL uint32_t* in, uint32_t n, UKL_GLOBAL uint32_t* abs, UKL_GLOBAL uint32_t* out, uint32_t most)
{
    // A stable sort by `kc`'s length, most first.
    for (uint32_t i = 1u; i < n; i++) {
        uint32_t x = in[i], j = i;
        while (j > 0u && m[in[j - 1u] + UKL_LANE_NKC] < m[x + UKL_LANE_NKC]) {
            in[j] = in[j - 1u];
            j--;
        }
        in[j] = x;
    }
    // Each lane kept (`abs` its place among the kept, plus one at its top bit) or a kept one's refusal.
    uint32_t kept = 0u;
    for (uint32_t i = 0u; i < n; i++) {
        uint32_t l = in[i], nk = m[l + UKL_LANE_NKC];
        uint32_t to = UKL_LANE_NONE;
        if (m[l + UKL_LANE_END] == 2u && nk > 0u) {
            uint32_t j = nk - 1u, lk = ukl_lane_kc(m, l);
            for (uint32_t q = 0u; q < i && to == UKL_LANE_NONE; q++) {
                if ((abs[q] >> 31) == 0u)
                    continue;
                uint32_t k = in[q], kk = ukl_lane_kc(m, k);
                if (m[k + UKL_LANE_ST0] != m[l + UKL_LANE_ST0] || m[k + UKL_LANE_NKC] <= j || m[kk + j] == m[lk + j])
                    continue;
                bool same = true;
                for (uint32_t t = 0u; t < j; t++)
                    same = same && m[kk + t] == m[lk + t];
                if (same)
                    to = q;
            }
        }
        if (to == UKL_LANE_NONE) {
            abs[i] = (1u << 31) | kept;
            kept++;
        } else {
            abs[i] = to;
        }
    }
    if (kept > most)
        return UKL_LANE_NONE;
    // The kept lanes anew, each with its refusals and those of the lanes it took.
    uint32_t o = 0u;
    for (uint32_t i = 0u; i < n; i++) {
        if ((abs[i] >> 31) == 0u)
            continue;
        uint32_t l = in[i];
        uint32_t nref = m[l + UKL_LANE_NREF];
        for (uint32_t q = i + 1u; q < n; q++)
            if (abs[q] == i)
                nref += m[in[q] + UKL_LANE_NREF] + 1u;
        uint32_t keep = ukl_lane_ref(m, l) - l;
        uint32_t at = ukl_lane_take(m, keep + UKL_LANE_REFUSAL * nref);
        if (at == UKL_LANE_NONE)
            return UKL_LANE_NONE;
        for (uint32_t k = 0u; k < keep; k++)
            m[at + k] = m[l + k];
        uint32_t r = at + keep;
        uint32_t own = ukl_lane_ref(m, l);
        for (uint32_t k = 0u; k < UKL_LANE_REFUSAL * m[l + UKL_LANE_NREF]; k++)
            m[r++] = m[own + k];
        for (uint32_t q = i + 1u; q < n; q++) {
            if (abs[q] != i)
                continue;
            uint32_t x = in[q], xr = ukl_lane_ref(m, x);
            for (uint32_t k = 0u; k < UKL_LANE_REFUSAL * m[x + UKL_LANE_NREF]; k++)
                m[r++] = m[xr + k];
            uint32_t j = m[x + UKL_LANE_NKC] - 1u;
            m[r++] = j;
            m[r++] = 1u << m[ukl_lane_kc(m, x) + j];
            m[r++] = m[x + UKL_LANE_ERR];
            m[r++] = m[x + UKL_LANE_NODES];
            m[r++] = m[x + UKL_LANE_HAS_LAST] != 0u ? m[x + UKL_LANE_LAST] : UKL_LANE_NONE;
        }
        // Sorted, each once.
        uint32_t base = at + keep;
        for (uint32_t a = 1u; a < nref; a++) {
            for (uint32_t b = a; b > 0u && ukl_lane_ref_cmp(m, base + UKL_LANE_REFUSAL * b, base + UKL_LANE_REFUSAL * (b - 1u)) < 0; b--) {
                for (uint32_t k = 0u; k < UKL_LANE_REFUSAL; k++) {
                    uint32_t t = m[base + UKL_LANE_REFUSAL * b + k];
                    m[base + UKL_LANE_REFUSAL * b + k] = m[base + UKL_LANE_REFUSAL * (b - 1u) + k];
                    m[base + UKL_LANE_REFUSAL * (b - 1u) + k] = t;
                }
            }
        }
        uint32_t w = 0u;
        for (uint32_t a = 0u; a < nref; a++) {
            if (w > 0u && ukl_lane_ref_cmp(m, base + UKL_LANE_REFUSAL * a, base + UKL_LANE_REFUSAL * (w - 1u)) == 0)
                continue;
            for (uint32_t k = 0u; k < UKL_LANE_REFUSAL; k++)
                m[base + UKL_LANE_REFUSAL * w + k] = m[base + UKL_LANE_REFUSAL * a + k];
            w++;
        }
        m[at + UKL_LANE_NREF] = w;
        out[o++] = at;
    }
    return o;
}
