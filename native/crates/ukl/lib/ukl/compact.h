// ukl/compact.h: stream compaction (decision 0440 §5): the values whose flag is set, in order, packed from 0. The
// host plan (`rerail_gpu::library::compact_u32`) scans the flags (0 or 1) into `offsets` with ukl/scan.h, then
// scatters with this kernel; `offsets[n]` is how many were kept.

#pragma once
#include <ukl/scan.h>

typedef struct UklCompactArgs {
    uint32_t n;
} UklCompactArgs;
UKL_REFLECT_STRUCT(UklCompactArgs, n)

// Value `i` goes to its slot when its flag set (its offset steps).
UKL_INLINE void ukl_compact_one(UKL_GLOBAL const uint32_t* values, UKL_GLOBAL const uint32_t* offsets, UKL_GLOBAL uint32_t* out, uint32_t i)
{
    uint32_t at = offsets[i];
    if (offsets[i + 1u] != at)
        out[at] = values[i];
}

DEFINE_KERNEL2(ukl, ukl_compact_u32,
               ARGS,     UklCompactArgs, args,    ,
               ROBUFFER, uint32_t,       values,  "[n]",
               ROBUFFER, uint32_t,       offsets, "[n + 1]",
               RWBUFFER, uint32_t,       out,     "[offsets[n]]",
               GID0,     uint32_t,       i,       )
{
    if (i < args.n)
        ukl_compact_one(values, offsets, out, i);
}
