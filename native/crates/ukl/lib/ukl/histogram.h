// ukl/histogram.h: counting codes in threadgroup memory (decision 0440 §5; the gpu lane's `hist` and count table,
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

typedef struct UklHistogramArgs {
    // Codes counted: `0..width`. A code at or above it sets `bad`.
    uint32_t width;
} UklHistogramArgs;
UKL_REFLECT_STRUCT(UklHistogramArgs, width)

// The counting over each code type the kernels take: `ukl_histogram_part_u16`, `ukl_histogram_part_u32`, ...
#define UKL_HIST_NAME(f) f##_u16
#define UKL_HIST_CODE uint16_t
#include <ukl/body/histogram_part.h>
#define UKL_HIST_NAME(f) f##_u32
#define UKL_HIST_CODE uint32_t
#include <ukl/body/histogram_part.h>

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
    ukl_histogram_part_u16(codes, offsets, H, bad, args.width, table, p, lid, lsz);
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
    ukl_histogram_part_u32(codes, offsets, H, bad, args.width, table, p, lid, lsz);
}
