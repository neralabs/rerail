// ukl/histogram.h: counting codes in threadgroup memory (decision 0400 §5; the gpu lane's `hist` and count table,
// accel/src/gpu/table.metal).
//
// A table of counters in threadgroup memory covers a window of codes; a range of codes is counted one window at a
// time, so the cost grows with the code range, never the counts. The pieces are threadgroup calls (every thread calls
// them); `ukl_histogram_u16`/`_u32` are the kernels: a partition a threadgroup.

#pragma once
#include <ukl/group.h>

// Counters a threadgroup table holds: 8 KB of u32.
#define UKL_HIST_TABLE 2048u

// `t[0..len]` = `v`, the threads in turn.
UKL_INLINE void ukl_tg_fill(UKL_LOCAL uint32_t* t, uint32_t len, uint32_t v, uint32_t lid, uint32_t lsz)
{
    for (uint32_t i = lid; i < len; i += lsz)
        t[i] = v;
}

// Adds to `table[c - base]` each code `c` of `codes[a..b]` in `base..base + len`; whether any code was at or above
// `limit` (seen by this thread).
template <typename C>
UKL_INLINE bool ukl_tg_count_window(UKL_LOCAL uint32_t* table, UKL_GLOBAL const C* codes, uint32_t a, uint32_t b, uint32_t base, uint32_t len,
                                    uint32_t limit, uint32_t lid, uint32_t lsz)
{
    bool over = false;
    for (uint32_t e = a + lid; e < b; e += lsz) {
        uint32_t code = (uint32_t)codes[e];
        uint32_t c = code - base;
        if (c < len)
            ukl_atomic_add_local(&table[c], 1u);
        over = over || code >= limit;
    }
    return over;
}

struct UklHistogramArgs {
    // Codes counted: `0..width`. A code at or above it sets `bad`.
    uint32_t width;
};
UKL_REFLECT_STRUCT(UklHistogramArgs, width)

// Partition `p`'s codes (`codes[offsets[p]..offsets[p + 1]]`) counted into `H[p * width + c]`.
template <typename C>
UKL_INLINE void ukl_histogram_part(UKL_GLOBAL const C* codes, UKL_GLOBAL const uint32_t* offsets, UKL_GLOBAL uint32_t* H, UKL_GLOBAL uint32_t* bad,
                                   uint32_t width, UKL_LOCAL uint32_t* table, uint32_t p, uint32_t lid, uint32_t lsz)
{
    uint32_t a = offsets[p], b = offsets[p + 1u];
    UKL_GLOBAL uint32_t* out = H + (uint64_t)p * width;
    for (uint32_t base = 0u; base < width; base += UKL_HIST_TABLE) {
        uint32_t len = width - base < UKL_HIST_TABLE ? width - base : UKL_HIST_TABLE;
        ukl_tg_fill(table, len, 0u, lid, lsz);
        ukl_tg_barrier();
        if (ukl_tg_count_window(table, codes, a, b, base, len, width, lid, lsz) && base == 0u)
            ukl_atomic_store(bad, 1u);
        ukl_tg_barrier();
        for (uint32_t i = lid; i < len; i += lsz)
            out[base + i] = table[i];
        ukl_tg_barrier();
    }
}

DEFINE_KERNEL2(ukl, ukl_histogram_u16,
               ARGS,     UklHistogramArgs, args,    ,
               ROBUFFER, uint16_t,         codes,   "[offsets[parts]]",
               ROBUFFER, uint32_t,         offsets, "[parts + 1]",
               RWBUFFER, uint32_t,         H,       "[parts * width]",
               RWBUFFER, uint32_t,         bad,     "[1]",
               SHARED,   uint32_t,         table,   UKL_HIST_TABLE,
               TGID0,    uint32_t,         p,       ,
               LID0,     uint32_t,         lid,     ,
               LSZ0,     uint32_t,         lsz,     )
{
    ukl_histogram_part(codes, offsets, H, bad, args.width, table, p, lid, lsz);
}

DEFINE_KERNEL2(ukl, ukl_histogram_u32,
               ARGS,     UklHistogramArgs, args,    ,
               ROBUFFER, uint32_t,         codes,   "[offsets[parts]]",
               ROBUFFER, uint32_t,         offsets, "[parts + 1]",
               RWBUFFER, uint32_t,         H,       "[parts * width]",
               RWBUFFER, uint32_t,         bad,     "[1]",
               SHARED,   uint32_t,         table,   UKL_HIST_TABLE,
               TGID0,    uint32_t,         p,       ,
               LID0,     uint32_t,         lid,     ,
               LSZ0,     uint32_t,         lsz,     )
{
    ukl_histogram_part(codes, offsets, H, bad, args.width, table, p, lid, lsz);
}
