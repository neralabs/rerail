// ukl/delta/par.h: what a delta operation's phases share to run over the whole grid (decision 0686): a threadgroup's
// scan of a block's words, and a sort as phases: each chunk of `DELTA_CHUNK` ids ranked, a lane an id (its place is
// how many of its chunk are less), then merge passes of a lane an id (its place in the pair of runs it is merged into
// is its place in its own run and how many of each other run are less: binary searches).
#pragma once
#include <ukl/delta/op.h>

// The exclusive scan of the `n` words at byte `at` of `b`, in place, by one threadgroup (`lsz` words a round, each
// thread its own), and their total. Every thread of the group calls it.
UKL_INLINE uint32_t delta_group_scan(Blk b, uint32_t at, uint32_t n, uint32_t lid, uint32_t lsz, UKL_LOCAL uint32_t* scratch __UKL_PARAMS)
{
    uint32_t run = 0u;
    for (uint32_t base = 0u; base < n; base += lsz) {
        uint32_t i = base + lid, v = i < n ? U32(b, at + 4u * i) : 0u;
        UklScan s = ukl_tg_exclusive_sum(v, lid, lsz, scratch __UKL_ARGS);
        if (i < n) U32(b, at + 4u * i) = run + s.exclusive;
        run += s.total;
    }
    return run;
}

// A sort as phases, of ids `0..n` in place in an array of a block, by `LESS(c, x, y)` (a strict total order of
// ids): `DELTA_CHUNK_RANK` places each id in its chunk of `DELTA_CHUNK` (a lane an id: its chunk's start and the ids
// of its chunk less than it), then each merge pass (`DELTA_MERGE_PLACE`, a lane an element) merges each `DELTA_FAN`
// runs `width` wide into one, in the other array, `width` growing `DELTA_FAN` times a turn (`delta_sort_items`,
// `delta_sort_again`: its words). A sort of up to 2^30 ids takes 6 turns.
#define DELTA_CHUNK 64u
#define DELTA_FAN 16u

#define DELTA_CHUNK_RANK(name, Ctx, LESS)                                                                           \
    UKL_INLINE uint32_t name(Ctx c, uint32_t n, uint32_t i)                                                         \
    {                                                                                                               \
        uint32_t lo = i & ~(DELTA_CHUNK - 1u), hi = lo + DELTA_CHUNK < n ? lo + DELTA_CHUNK : n, r = lo;            \
        for (uint32_t j = lo; j < hi; j++) r += j != i && LESS(c, j, i) ? 1u : 0u;                                  \
        return r;                                                                                                   \
    }

// A sort's words at `at` of `b`: its runs' width, and which array holds them (0: the first). Set before its passes.
UKL_INLINE void delta_sort_start(Blk b, uint32_t at)
{
    U32(b, at) = DELTA_CHUNK;
    U32(b, at + 4u) = 0u;
}
// A pass's items: every element while the runs are narrower than the whole.
UKL_INLINE uint32_t delta_sort_items(Blk b, uint32_t at, uint32_t n) { return U32(b, at) < n ? n : 0u; }
// After a pass: the runs `DELTA_FAN` times as wide, in the other array; whether another pass is due.
UKL_INLINE bool delta_sort_again(Blk b, uint32_t at, uint32_t n)
{
    uint32_t width = U32(b, at);
    if (width >= n) return false;
    U32(b, at) = DELTA_FAN * width;
    U32(b, at + 4u) ^= 1u;
    return DELTA_FAN * width < n;
}
// Which of the arrays `a0`, `a1` the runs are in, and the other.
UKL_INLINE uint32_t delta_sort_from(Blk b, uint32_t at, uint32_t a0, uint32_t a1) { return U32(b, at + 4u) != 0u ? a1 : a0; }
UKL_INLINE uint32_t delta_sort_to(Blk b, uint32_t at, uint32_t a0, uint32_t a1) { return U32(b, at + 4u) != 0u ? a0 : a1; }

// `name(c, b, at, n, width, i)`: where the id at place `i` of the array at byte `at` of `b` (`n` ids, sorted in
// runs `width` wide by `LESS(c, x, y)`, a strict total order) goes when each `DELTA_FAN` runs are merged: its place
// in its own run, and each other run's ids less than it (a binary search). `name_pass` moves it there, into the other
// array (the sort's words at `words` of `wb`).
#define DELTA_MERGE_PLACE(name, Ctx, LESS)                                                                          \
    UKL_INLINE uint32_t name(Ctx c, Blk b, uint32_t at, uint32_t n, uint32_t width, uint32_t i)                     \
    {                                                                                                               \
        uint32_t span = width * DELTA_FAN, lo = i / span * span, own = i / width * width;                           \
        uint32_t x = U32(b, at + 4u * i), place = lo + (i - own), end = n - lo < span ? n : lo + span;              \
        for (uint32_t r = lo; r < end; r += width) {                                                                \
            if (r == own) continue;                                                                                 \
            uint32_t a = r, e = end - r < width ? end : r + width;                                                  \
            while (a < e) {                                                                                         \
                uint32_t m = (a + e) / 2u;                                                                          \
                if (LESS(c, U32(b, at + 4u * m), x)) a = m + 1u; else e = m;                                        \
            }                                                                                                       \
            place += a - r;                                                                                         \
        }                                                                                                           \
        return place;                                                                                               \
    }                                                                                                               \
    UKL_INLINE void name##_pass(Ctx c, Blk wb, uint32_t words, Blk b, uint32_t a0, uint32_t a1, uint32_t n, uint32_t i) \
    {                                                                                                               \
        uint32_t from = delta_sort_from(wb, words, a0, a1), to = delta_sort_to(wb, words, a0, a1);                  \
        U32(b, to + 4u * name(c, b, from, n, U32(wb, words), i)) = U32(b, from + 4u * i);                           \
    }
