// ukl/middle.h: tier 2 from the middle on the device (decision 0726), as its Rust's bracket scan and frame scan: a
// chunk's closers it cannot match kept as anti-opens, the mirror of its opens, and cancelled against the opens before
// them by a scan, so each chunk's place and levels are known before its one walk; then the walks' spans of the
// writer's frames, whose anti-frames cancel the frames before them by a second scan.
//
// Every value is a record of words in a pool, taken by an atomic bump of the pool's first word (its second its size):
//
//   a word (`Word`):     0 its anti-opens' count, 1 its opens', 2 broken; then the anti-opens' kinds in file order,
//                        then the opens' outermost first;
//   kinds (`Kinds`):     0 their count; then the open levels' kinds outermost first, the root's at the bottom;
//   a span (`Span`):     0-3 its exit (`i`, `e`, `depth`, `st`), 4 its nodes, 5 its end (0 none, 1 the file's end, 2
//                        its refusal), 6 the refusal's byte, 7 `fkc`, 8 `touch`'s length, 9 `pf`'s, 10-11 its last
//                        start (low, high), 12 whether it has one; then `touch` (two words each: children, last or
//                        `UKL_MID_NONE`) and `pf` (`UKL_MID_FRAME` words each: node, children, last, start's low and
//                        high words);
//   a state (`At`):      0-3 its entry, 4 the nodes before, 5 its end (as a span's), 6 its refusal's byte, 7-8 the start
//                        before (low, high), 9 its levels' count, 10 its frames'; then its levels' places, the innermost
//                        last, and its frames, outermost first.
//
// A value no lane of the Rust's has (`None`) is `UKL_MID_NONE`; one past the pool's room `UKL_MID_FULL`, and the run is
// made again with more (the engine's `room`).
#pragma once
#include <ukl/backend.h>

#define UKL_MID_NONE 4294967295u
#define UKL_MID_FULL 4294967294u
#define UKL_MID_FRAME 5u
#define UKL_SPAN_HEAD 13u
#define UKL_AT_HEAD 11u

typedef UKL_GLOBAL uint32_t* UklMidPool;

// `n` words of the pool, or `UKL_MID_FULL` past its size.
UKL_INLINE uint32_t ukl_mid_take(UklMidPool m, uint32_t n)
{
    uint32_t at = ukl_atomic_add(m, n);
    return at + n > m[1] ? UKL_MID_FULL : at;
}

// A node number of a range written after `base` nodes, `UKL_MID_NONE` kept.
UKL_INLINE uint32_t ukl_mid_shift(uint32_t x, uint32_t base) { return x == UKL_MID_NONE ? x : x + base; }

// ---- the bracket word ---------------------------------------------------------------------------------------------

// A closer of kind `k` after the word in `w[0..]` (its opens at `w[3 + na..]`, `*no` of them): it cancels the innermost
// open, or is an anti-open (written at `w[3 + *na]`, the opens moved up a word). The Rust's `Word::cancel`.
UKL_INLINE void ukl_word_cancel(UklMidPool m, uint32_t w, uint32_t k)
{
    uint32_t na = m[w], no = m[w + 1u];
    if (no > 0u) {
        if (m[w + 3u + na + no - 1u] != k)
            m[w + 2u] = 1u;
        m[w + 1u] = no - 1u;
        return;
    }
    m[w + 3u + na] = k;
    m[w] = na + 1u;
}

// Word `a` then word `b` (`Word::then`): `b`'s anti-opens cancelled against `a`'s opens, innermost first. Its offset,
// or `UKL_MID_FULL`.
UKL_INLINE uint32_t ukl_word_then(UklMidPool m, uint32_t a, uint32_t b)
{
    uint32_t ana = m[a], ano = m[a + 1u], bna = m[b], bno = m[b + 1u];
    uint32_t at = ukl_mid_take(m, 3u + ana + ano + bna + bno);
    if (at == UKL_MID_FULL)
        return at;
    m[at] = ana;
    m[at + 1u] = ano;
    m[at + 2u] = m[a + 2u] | m[b + 2u];
    for (uint32_t k = 0u; k < ana + ano; k++)
        m[at + 3u + k] = m[a + 3u + k];
    // Each anti-open of `b` cancels an open of `a` or follows `a`'s anti-opens; `a`'s opens are above them, so an
    // anti-open written moves no open: only once every open of `a` is cancelled.
    for (uint32_t k = 0u; k < bna; k++)
        ukl_word_cancel(m, at, m[b + 3u + k]);
    uint32_t base = at + 3u + m[at] + m[at + 1u];
    for (uint32_t k = 0u; k < bno; k++)
        m[base + k] = m[b + 3u + bna + k];
    m[at + 1u] += bno;
    return at;
}

// The kinds word `w` leaves after kinds `a` (`kinds_after`): its anti-opens cancel `a`'s innermost levels (the root
// closes to nothing), its opens follow. `UKL_MID_NONE` where they do not meet, or `w` is broken.
UKL_INLINE uint32_t ukl_kinds_after(UklMidPool m, uint32_t a, uint32_t w, uint32_t root)
{
    if (a >= UKL_MID_FULL)
        return a;
    if (m[w + 2u] != 0u)
        return UKL_MID_NONE;
    uint32_t n = m[a], na = m[w], no = m[w + 1u];
    if (na > n)
        return UKL_MID_NONE;
    for (uint32_t k = 0u; k < na; k++) {
        uint32_t o = m[a + 1u + n - 1u - k];
        if (o != m[w + 3u + k] || o == root)
            return UKL_MID_NONE;
    }
    uint32_t at = ukl_mid_take(m, 1u + n - na + no);
    if (at == UKL_MID_FULL)
        return at;
    m[at] = n - na + no;
    for (uint32_t k = 0u; k < n - na; k++)
        m[at + 1u + k] = m[a + 1u + k];
    for (uint32_t k = 0u; k < no; k++)
        m[at + 1u + n - na + k] = m[w + 3u + na + k];
    return at;
}

// Kinds `a` as word `w`'s range reads them (`kinds_keep`): its anti-opens' levels and the innermost one.
UKL_INLINE uint32_t ukl_kinds_keep(UklMidPool m, uint32_t a, uint32_t w)
{
    if (a >= UKL_MID_FULL)
        return a;
    uint32_t n = m[a], keep = m[w] + 1u;
    if (n <= keep)
        return a;
    uint32_t at = ukl_mid_take(m, 1u + keep);
    if (at == UKL_MID_FULL)
        return at;
    m[at] = keep;
    for (uint32_t k = 0u; k < keep; k++)
        m[at + 1u + k] = m[a + 1u + n - keep + k];
    return at;
}

// ---- the writer's frames ------------------------------------------------------------------------------------------

UKL_INLINE uint32_t ukl_span_touch(UklMidPool m, uint32_t s) { return s + UKL_SPAN_HEAD; }
UKL_INLINE uint32_t ukl_span_pf(UklMidPool m, uint32_t s) { return s + UKL_SPAN_HEAD + 2u * m[s + 8u]; }

// Span `a` then span `b` (`Span::then`): `b`'s anti-frames cancel the frames `a` leaves open, the rest closed below
// `a`'s entry; what `b` wrote into the one it is then in, that frame's. A span that ended goes on as itself.
UKL_INLINE uint32_t ukl_span_then(UklMidPool m, uint32_t a, uint32_t b)
{
    if (m[a + 5u] != 0u)
        return a;
    uint32_t n = m[a + 9u], bfkc = m[b + 7u];
    uint32_t fm = bfkc < n ? bfkc : n;
    uint32_t ant = m[a + 8u], bnt = m[b + 8u], bnpf = m[b + 9u];
    uint32_t nt = bfkc < n ? ant : ant + (bnt - n - 1u);
    uint32_t npf = n - fm + bnpf;
    uint32_t at = ukl_mid_take(m, UKL_SPAN_HEAD + 2u * nt + UKL_MID_FRAME * npf);
    if (at == UKL_MID_FULL)
        return at;
    uint32_t base = m[a + 4u];
    for (uint32_t k = 0u; k < 4u; k++)
        m[at + k] = m[b + k];
    m[at + 4u] = base + m[b + 4u];
    m[at + 5u] = m[b + 5u];
    m[at + 6u] = m[b + 6u];
    m[at + 7u] = m[a + 7u] + bfkc - fm;
    m[at + 8u] = nt;
    m[at + 9u] = npf;
    uint32_t last = m[b + 12u] != 0u ? b : a;
    m[at + 10u] = m[last + 10u];
    m[at + 11u] = m[last + 11u];
    m[at + 12u] = m[last + 12u];
    uint32_t w = at + UKL_SPAN_HEAD, touch = w, at_ = ukl_span_touch(m, a), bt = ukl_span_touch(m, b);
    for (uint32_t k = 0u; k < 2u * ant; k++)
        m[w++] = m[at_ + k];
    if (bfkc >= n) {
        uint32_t s = touch + 2u * (ant - 1u), t = bt + 2u * n;
        m[s] += m[t];
        if (m[t + 1u] != UKL_MID_NONE)
            m[s + 1u] = m[t + 1u] + base;
        for (uint32_t k = n + 1u; k < bnt; k++) {
            m[w++] = m[bt + 2u * k];
            m[w++] = ukl_mid_shift(m[bt + 2u * k + 1u], base);
        }
    }
    uint32_t pf = w, af = ukl_span_pf(m, a), bf = ukl_span_pf(m, b);
    for (uint32_t k = 0u; k < UKL_MID_FRAME * (n - fm); k++)
        m[w++] = m[af + k];
    if (bfkc < n) {
        uint32_t f = pf + UKL_MID_FRAME * (n - fm - 1u), t = bt + 2u * bfkc;
        m[f + 1u] += m[t];
        if (m[t + 1u] != UKL_MID_NONE)
            m[f + 2u] = m[t + 1u] + base;
    }
    for (uint32_t k = 0u; k < bnpf; k++) {
        uint32_t f = bf + UKL_MID_FRAME * k;
        m[w++] = m[f] + base;
        m[w++] = m[f + 1u];
        m[w++] = ukl_mid_shift(m[f + 2u], base);
        m[w++] = m[f + 3u];
        m[w++] = m[f + 4u];
    }
    return at;
}

UKL_INLINE uint32_t ukl_at_new(UklMidPool m, uint32_t nret, uint32_t nframes)
{
    uint32_t at = ukl_mid_take(m, UKL_AT_HEAD + nret + UKL_MID_FRAME * nframes);
    if (at == UKL_MID_FULL)
        return at;
    for (uint32_t k = 0u; k < UKL_AT_HEAD; k++)
        m[at + k] = 0u;
    m[at + 9u] = nret;
    m[at + 10u] = nframes;
    return at;
}

// The state span `s` leaves after state `a` (`at_after`): its anti-frames cancel `a`'s innermost frames, its opens
// follow, its place its exit. `UKL_MID_NONE` where `a` holds fewer frames than it closes.
UKL_INLINE uint32_t ukl_at_after(UklMidPool m, uint32_t a, uint32_t s)
{
    if (a >= UKL_MID_FULL || m[a + 5u] != 0u)
        return a;
    uint32_t fkc = m[s + 7u], npf = m[s + 9u], af = m[a + 10u], nret = m[a + 9u];
    if (fkc > af)
        return UKL_MID_NONE;
    uint32_t nf = af - fkc;
    uint32_t at = ukl_at_new(m, 0u, nf + npf);
    if (at == UKL_MID_FULL)
        return at;
    uint32_t base = m[a + 4u];
    for (uint32_t k = 0u; k < 4u; k++)
        m[at + k] = m[s + k];
    m[at + 4u] = base + m[s + 4u];
    m[at + 5u] = m[s + 5u];
    m[at + 6u] = m[s + 6u];
    bool has = m[s + 12u] != 0u;
    m[at + 7u] = has ? m[s + 10u] : m[a + 7u];
    m[at + 8u] = has ? m[s + 11u] : m[a + 8u];
    uint32_t w = at + UKL_AT_HEAD, afr = a + UKL_AT_HEAD + nret;
    for (uint32_t k = 0u; k < UKL_MID_FRAME * nf; k++)
        m[w++] = m[afr + k];
    if (nf > 0u) {
        uint32_t f = w - UKL_MID_FRAME, t = ukl_span_touch(m, s) + 2u * fkc;
        m[f + 1u] += m[t];
        if (m[t + 1u] != UKL_MID_NONE)
            m[f + 2u] = m[t + 1u] + base;
    }
    uint32_t sf = ukl_span_pf(m, s);
    for (uint32_t k = 0u; k < npf; k++) {
        uint32_t f = sf + UKL_MID_FRAME * k;
        m[w++] = m[f] + base;
        m[w++] = m[f + 1u];
        m[w++] = ukl_mid_shift(m[f + 2u], base);
        m[w++] = m[f + 3u];
        m[w++] = m[f + 4u];
    }
    return at;
}

// State `a` as span `s` reads it (`at_keep`): the frames it closes and the one it then writes into.
UKL_INLINE uint32_t ukl_at_keep(UklMidPool m, uint32_t a, uint32_t s)
{
    if (a >= UKL_MID_FULL || m[a + 5u] != 0u)
        return a;
    uint32_t nf = m[a + 10u], keep = m[s + 7u] + 1u, nret = m[a + 9u];
    if (nf <= keep)
        return a;
    uint32_t at = ukl_at_new(m, nret, keep);
    if (at == UKL_MID_FULL)
        return at;
    for (uint32_t k = 0u; k < 9u; k++)
        m[at + k] = m[a + k];
    uint32_t w = at + UKL_AT_HEAD;
    for (uint32_t k = 0u; k < nret; k++)
        m[w++] = m[a + UKL_AT_HEAD + k];
    uint32_t afr = a + UKL_AT_HEAD + nret + UKL_MID_FRAME * (nf - keep);
    for (uint32_t k = 0u; k < UKL_MID_FRAME * keep; k++)
        m[w++] = m[afr + k];
    return at;
}
