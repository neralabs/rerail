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

// `then`'s answer where the lanes do not meet, and any answer past the pool's room.
#define UKL_LANE_FULL 4294967294u

// A lane's last start as a refusal keeps it: its low word, or `UKL_LANE_NONE`.
UKL_INLINE uint32_t ukl_lane_start32(UklLanePool m, uint32_t l) { return m[l + UKL_LANE_HAS_LAST] != 0u ? m[l + UKL_LANE_LAST] : UKL_LANE_NONE; }

// A node number of a range written after `base` nodes, `UKL_LANE_NONE` kept.
UKL_INLINE uint32_t ukl_lane_shift(uint32_t x, uint32_t base) { return x == UKL_LANE_NONE ? x : x + base; }

// Lane `a` ended where refusal `x` (a word offset) holds (`Lane::refused`).
UKL_INLINE uint32_t ukl_lane_refused(UklLanePool m, uint32_t a, uint32_t x)
{
    uint32_t nkc = m[a + UKL_LANE_NKC], nref = m[a + UKL_LANE_NREF];
    uint32_t at = ukl_lane_take(m, UKL_LANE_HEAD + nkc + 2u + UKL_LANE_REFUSAL * nref);
    if (at == UKL_LANE_NONE)
        return UKL_LANE_FULL;
    for (uint32_t k = 0u; k < UKL_LANE_HEAD; k++)
        m[at + k] = 0u;
    m[at + UKL_LANE_ST0] = m[a + UKL_LANE_ST0];
    m[at + UKL_LANE_NKC] = nkc;
    m[at + UKL_LANE_NODES] = m[a + UKL_LANE_NODES] + m[x + 3u];
    m[at + UKL_LANE_END] = 2u;
    m[at + UKL_LANE_ERR] = m[x + 2u];
    m[at + UKL_LANE_NTOUCH] = 1u;
    if (m[x + 4u] != UKL_LANE_NONE) {
        m[at + UKL_LANE_LAST] = m[x + 4u];
        m[at + UKL_LANE_HAS_LAST] = 1u;
    } else if (m[a + UKL_LANE_HAS_LAST] != 0u) {
        m[at + UKL_LANE_LAST] = m[a + UKL_LANE_LAST];
        m[at + UKL_LANE_LAST + 1u] = m[a + UKL_LANE_LAST + 1u];
        m[at + UKL_LANE_HAS_LAST] = 1u;
    }
    m[at + UKL_LANE_NREF] = nref;
    uint32_t w = at + UKL_LANE_HEAD, ak = ukl_lane_kc(m, a);
    for (uint32_t k = 0u; k < nkc; k++)
        m[w++] = m[ak + k];
    m[w++] = 0u;
    m[w++] = UKL_LANE_NONE;
    uint32_t ar = ukl_lane_ref(m, a);
    for (uint32_t k = 0u; k < UKL_LANE_REFUSAL * nref; k++)
        m[w++] = m[ar + k];
    return at;
}

// Lane `a` then lane `b`, entered where `a` leaves (`then`): `b`'s closed levels matched against those `a` leaves open,
// the rest closed below `a`'s entry. `UKL_LANE_NONE` where they do not meet, `UKL_LANE_FULL` past the pool's room.
UKL_INLINE uint32_t ukl_lane_then(UklLanePool m, uint32_t a, uint32_t b)
{
    if (m[a + UKL_LANE_END] != 0u)
        return a;
    if (m[b + UKL_LANE_ST0] != m[a + UKL_LANE_EXIT + 3u])
        return UKL_LANE_NONE;
    uint32_t anp = m[a + UKL_LANE_NP], bnk = m[b + UKL_LANE_NKC], bnr = m[b + UKL_LANE_NREF];
    uint32_t mm = bnk < anp ? bnk : anp;
    uint32_t ap = ukl_lane_p(m, a), bk = ukl_lane_kc(m, b), br = ukl_lane_ref(m, b);
    uint32_t r = 0u;
    for (uint32_t j = 0u; j < mm; j++) {
        uint32_t place = m[ap + anp - 1u - j];
        while (r < bnr && m[br + UKL_LANE_REFUSAL * r] == j) {
            if (((m[br + UKL_LANE_REFUSAL * r + 1u] >> place) & 1u) != 0u)
                return ukl_lane_refused(m, a, br + UKL_LANE_REFUSAL * r);
            r++;
        }
        if (m[bk + j] != place)
            return UKL_LANE_NONE;
    }
    uint32_t ank = m[a + UKL_LANE_NKC], anr = m[a + UKL_LANE_NREF], bnp = m[b + UKL_LANE_NP];
    uint32_t n = m[a + UKL_LANE_NPF], bfkc = m[b + UKL_LANE_FKC];
    uint32_t fm = bfkc < n ? bfkc : n;
    uint32_t ant = m[a + UKL_LANE_NTOUCH], bnt = m[b + UKL_LANE_NTOUCH], bnpf = m[b + UKL_LANE_NPF];
    uint32_t nkc = ank + bnk - mm, np = anp - mm + bnp;
    uint32_t nt = bfkc < n ? ant : ant + (bnt - n - 1u);
    uint32_t npf = n - fm + bnpf, nref = anr + bnr - r;
    uint32_t at = ukl_lane_take(m, UKL_LANE_HEAD + nkc + np + 2u * nt + UKL_LANE_FRAME * npf + UKL_LANE_REFUSAL * nref);
    if (at == UKL_LANE_NONE)
        return UKL_LANE_FULL;
    uint32_t base = m[a + UKL_LANE_NODES];
    m[at + UKL_LANE_ST0] = m[a + UKL_LANE_ST0];
    m[at + UKL_LANE_NKC] = nkc;
    m[at + UKL_LANE_NP] = np;
    for (uint32_t k = 0u; k < 4u; k++)
        m[at + UKL_LANE_EXIT + k] = m[b + UKL_LANE_EXIT + k];
    m[at + UKL_LANE_NODES] = base + m[b + UKL_LANE_NODES];
    m[at + UKL_LANE_END] = m[b + UKL_LANE_END];
    m[at + UKL_LANE_ERR] = m[b + UKL_LANE_ERR];
    m[at + UKL_LANE_FKC] = m[a + UKL_LANE_FKC] + bfkc - fm;
    m[at + UKL_LANE_NTOUCH] = nt;
    m[at + UKL_LANE_NPF] = npf;
    uint32_t last = b;
    if (m[b + UKL_LANE_HAS_LAST] == 0u)
        last = a;
    m[at + UKL_LANE_LAST] = m[last + UKL_LANE_LAST];
    m[at + UKL_LANE_LAST + 1u] = m[last + UKL_LANE_LAST + 1u];
    m[at + UKL_LANE_HAS_LAST] = m[last + UKL_LANE_HAS_LAST];
    m[at + UKL_LANE_NREF] = nref;
    uint32_t w = at + UKL_LANE_HEAD;
    // The levels: `a`'s closed then the rest of `b`'s; `a`'s open but those `b` closed, then `b`'s.
    uint32_t ak = ukl_lane_kc(m, a);
    for (uint32_t k = 0u; k < ank; k++)
        m[w++] = m[ak + k];
    for (uint32_t k = mm; k < bnk; k++)
        m[w++] = m[bk + k];
    for (uint32_t k = 0u; k < anp - mm; k++)
        m[w++] = m[ap + k];
    uint32_t bp = ukl_lane_p(m, b);
    for (uint32_t k = 0u; k < bnp; k++)
        m[w++] = m[bp + k];
    // The frames: `b`'s closed ones matched against those `a` leaves open, the rest closed below `a`'s entry; what `b`
    // wrote into the one it is then in, that frame's.
    uint32_t touch = w, at_ = ukl_lane_touch(m, a), bt = ukl_lane_touch(m, b);
    for (uint32_t k = 0u; k < 2u * ant; k++)
        m[w++] = m[at_ + k];
    if (bfkc >= n) {
        uint32_t s = touch + 2u * (ant - 1u), t = bt + 2u * n;
        m[s] += m[t];
        if (m[t + 1u] != UKL_LANE_NONE)
            m[s + 1u] = m[t + 1u] + base;
        for (uint32_t k = n + 1u; k < bnt; k++) {
            m[w++] = m[bt + 2u * k];
            m[w++] = ukl_lane_shift(m[bt + 2u * k + 1u], base);
        }
    }
    uint32_t pf = w, af = ukl_lane_pf(m, a), bf = ukl_lane_pf(m, b);
    for (uint32_t k = 0u; k < UKL_LANE_FRAME * (n - fm); k++)
        m[w++] = m[af + k];
    if (bfkc < n) {
        uint32_t f = pf + UKL_LANE_FRAME * (n - fm - 1u), t = bt + 2u * bfkc;
        m[f + 1u] += m[t];
        if (m[t + 1u] != UKL_LANE_NONE)
            m[f + 2u] = m[t + 1u] + base;
    }
    for (uint32_t k = 0u; k < bnpf; k++) {
        uint32_t f = bf + UKL_LANE_FRAME * k;
        m[w++] = m[f] + base;
        m[w++] = m[f + 1u];
        m[w++] = ukl_lane_shift(m[f + 2u], base);
        m[w++] = m[f + 3u];
        m[w++] = m[f + 4u];
    }
    // The refusals: `a`'s, then the rest of `b`'s at the levels they are of the two.
    uint32_t ar = ukl_lane_ref(m, a);
    for (uint32_t k = 0u; k < UKL_LANE_REFUSAL * anr; k++)
        m[w++] = m[ar + k];
    uint32_t as = ukl_lane_start32(m, a);
    for (uint32_t k = r; k < bnr; k++) {
        uint32_t x = br + UKL_LANE_REFUSAL * k;
        m[w++] = ank + m[x] - mm;
        m[w++] = m[x + 1u];
        m[w++] = m[x + 2u];
        m[w++] = base + m[x + 3u];
        m[w++] = m[x + 4u] != UKL_LANE_NONE ? m[x + 4u] : as;
    }
    return at;
}

// Whether lanes `a` and `b` are the same, word for word.
UKL_INLINE bool ukl_lane_same(UklLanePool m, uint32_t a, uint32_t b)
{
    uint32_t n = ukl_lane_size(m, a);
    if (n != ukl_lane_size(m, b))
        return false;
    for (uint32_t k = 0u; k < n; k++)
        if (m[a + k] != m[b + k])
            return false;
    return true;
}

// A summary's header: its status (0 made), its lanes' count, their offsets (at most `most`).
// Summaries `a` then `b` (`compose`) into header `out`: each of `a`'s lanes then each of `b`'s, each lane once, folded.
// `raw` is `2 * most * most` words of scratch. Its status: 0, or 2 past the pool's room or `most`.
UKL_INLINE void ukl_lane_compose(UklLanePool m, UKL_GLOBAL const uint32_t* a, UKL_GLOBAL const uint32_t* b, UKL_GLOBAL uint32_t* raw, UKL_GLOBAL uint32_t* out, uint32_t most)
{
    if (a[0] != 0u || b[0] != 0u) {
        out[0] = a[0] != 0u ? a[0] : b[0];
        out[1] = 0u;
        return;
    }
    uint32_t n = 0u, cap = most * most;
    for (uint32_t i = 0u; i < a[1]; i++) {
        uint32_t la = a[2u + i];
        bool ended = m[la + UKL_LANE_END] != 0u;
        uint32_t nb = ended ? 1u : b[1];
        for (uint32_t j = 0u; j < nb; j++) {
            uint32_t l = ended ? la : ukl_lane_then(m, la, b[2u + j]);
            if (l == UKL_LANE_NONE)
                continue;
            if (l == UKL_LANE_FULL || n >= cap) {
                out[0] = 2u;
                out[1] = 0u;
                return;
            }
            bool seen = false;
            for (uint32_t k = 0u; k < n && !seen; k++)
                seen = ukl_lane_same(m, raw[k], l);
            if (!seen)
                raw[n++] = l;
        }
    }
    uint32_t k = ukl_lane_fold(m, raw, n, raw + cap, out + 2u, most);
    out[0] = k == UKL_LANE_NONE ? 2u : 0u;
    out[1] = k == UKL_LANE_NONE ? 0u : k;
}

// A walk's state at a range's start (`At`), a record of words in the pool: 0-3 its entry (`i`, `e`, `depth`, `st`),
// 4 the nodes before, 5 the file's end (as a lane's), 6 its refusal's byte, 7-8 the start before (low, high), 9 its
// levels' count, 10 its frames'; then its levels' places, the innermost last (a word each), and its frames, outermost
// first (`UKL_LANE_FRAME` words each).
#define UKL_AT_HEAD 11u

UKL_INLINE uint32_t ukl_at_new(UklLanePool m, uint32_t nret, uint32_t nframes)
{
    uint32_t at = ukl_lane_take(m, UKL_AT_HEAD + nret + UKL_LANE_FRAME * nframes);
    if (at == UKL_LANE_NONE)
        return UKL_LANE_FULL;
    for (uint32_t k = 0u; k < UKL_AT_HEAD; k++)
        m[at + k] = 0u;
    m[at + 9u] = nret;
    m[at + 10u] = nframes;
    return at;
}

// The state range `s` (a header) leaves after state `a` at its start (`apply`): the lane of `a`'s place whose closed
// levels are `a`'s. `UKL_LANE_NONE` where no lane meets it, `UKL_LANE_FULL` past the pool's room.
UKL_INLINE uint32_t ukl_lane_apply(UklLanePool m, UKL_GLOBAL const uint32_t* s, uint32_t a)
{
    if (m[a + 5u] != 0u)
        return a;
    uint32_t d = m[a + 9u], ar = a + UKL_AT_HEAD;
    uint32_t found = UKL_LANE_NONE;
    for (uint32_t i = 0u; i < s[1] && found == UKL_LANE_NONE; i++) {
        uint32_t l = s[2u + i], nk = m[l + UKL_LANE_NKC];
        if (m[l + UKL_LANE_ST0] != m[a + 3u] || nk > d)
            continue;
        uint32_t lk = ukl_lane_kc(m, l), lr = ukl_lane_ref(m, l), nr = m[l + UKL_LANE_NREF];
        uint32_t r = 0u;
        bool meets = true;
        for (uint32_t j = 0u; j < nk && meets; j++) {
            uint32_t place = m[ar + d - 1u - j];
            while (r < nr && m[lr + UKL_LANE_REFUSAL * r] == j) {
                uint32_t x = lr + UKL_LANE_REFUSAL * r;
                if (((m[x + 1u] >> place) & 1u) != 0u) {
                    uint32_t at = ukl_at_new(m, 0u, 0u);
                    if (at == UKL_LANE_FULL)
                        return at;
                    m[at + 4u] = m[a + 4u] + m[x + 3u];
                    m[at + 5u] = 2u;
                    m[at + 6u] = m[x + 2u];
                    m[at + 7u] = m[x + 4u] != UKL_LANE_NONE ? m[x + 4u] : m[a + 7u];
                    m[at + 8u] = m[x + 4u] != UKL_LANE_NONE ? 0u : m[a + 8u];
                    return at;
                }
                r++;
            }
            meets = m[lk + j] == place;
        }
        if (meets)
            found = l;
    }
    if (found == UKL_LANE_NONE)
        return UKL_LANE_NONE;
    uint32_t l = found, nk = m[l + UKL_LANE_NKC], np = m[l + UKL_LANE_NP], fkc = m[l + UKL_LANE_FKC], npf = m[l + UKL_LANE_NPF];
    uint32_t af = m[a + 10u];
    if (fkc > af)
        return UKL_LANE_NONE;
    uint32_t nf = af - fkc;
    uint32_t at = ukl_at_new(m, d - nk + np, nf + npf);
    if (at == UKL_LANE_FULL)
        return at;
    uint32_t base = m[a + 4u];
    for (uint32_t k = 0u; k < 4u; k++)
        m[at + k] = m[l + UKL_LANE_EXIT + k];
    m[at + 4u] = base + m[l + UKL_LANE_NODES];
    m[at + 5u] = m[l + UKL_LANE_END];
    m[at + 6u] = m[l + UKL_LANE_ERR];
    bool has = m[l + UKL_LANE_HAS_LAST] != 0u;
    m[at + 7u] = has ? m[l + UKL_LANE_LAST] : m[a + 7u];
    m[at + 8u] = has ? m[l + UKL_LANE_LAST + 1u] : m[a + 8u];
    uint32_t w = at + UKL_AT_HEAD;
    for (uint32_t k = 0u; k < d - nk; k++)
        m[w++] = m[ar + k];
    uint32_t lp = ukl_lane_p(m, l);
    for (uint32_t k = 0u; k < np; k++)
        m[w++] = m[lp + k];
    uint32_t afr = ar + d;
    for (uint32_t k = 0u; k < UKL_LANE_FRAME * nf; k++)
        m[w++] = m[afr + k];
    if (nf > 0u) {
        uint32_t f = w - UKL_LANE_FRAME, t = ukl_lane_touch(m, l) + 2u * fkc;
        m[f + 1u] += m[t];
        if (m[t + 1u] != UKL_LANE_NONE)
            m[f + 2u] = m[t + 1u] + base;
    }
    uint32_t lf = ukl_lane_pf(m, l);
    for (uint32_t k = 0u; k < npf; k++) {
        uint32_t f = lf + UKL_LANE_FRAME * k;
        m[w++] = m[f] + base;
        m[w++] = m[f + 1u];
        m[w++] = ukl_lane_shift(m[f + 2u], base);
        m[w++] = m[f + 3u];
        m[w++] = m[f + 4u];
    }
    return at;
}

// State `a` as range `s` reads it (`keep`): its innermost levels' places, as many as a lane of `s` closes below its
// start, and its innermost frames, as many as a lane closes and the one it then writes into. A new record where it
// drops any.
UKL_INLINE uint32_t ukl_lane_keep(UklLanePool m, uint32_t a, UKL_GLOBAL const uint32_t* s)
{
    if (m[a + 5u] != 0u)
        return a;
    uint32_t levels = 0u, frames = 0u;
    for (uint32_t i = 0u; i < s[1]; i++) {
        uint32_t l = s[2u + i];
        levels = m[l + UKL_LANE_NKC] > levels ? m[l + UKL_LANE_NKC] : levels;
        frames = m[l + UKL_LANE_FKC] + 1u > frames ? m[l + UKL_LANE_FKC] + 1u : frames;
    }
    uint32_t d = m[a + 9u], nf = m[a + 10u];
    uint32_t kd = d < levels ? d : levels, kf = nf < frames ? nf : frames;
    if (kd == d && kf == nf)
        return a;
    uint32_t at = ukl_at_new(m, kd, kf);
    if (at == UKL_LANE_FULL)
        return at;
    for (uint32_t k = 0u; k < 9u; k++)
        m[at + k] = m[a + k];
    uint32_t w = at + UKL_AT_HEAD, ar = a + UKL_AT_HEAD;
    for (uint32_t k = d - kd; k < d; k++)
        m[w++] = m[ar + k];
    for (uint32_t k = UKL_LANE_FRAME * (nf - kf); k < UKL_LANE_FRAME * nf; k++)
        m[w++] = m[ar + d + k];
    return at;
}
