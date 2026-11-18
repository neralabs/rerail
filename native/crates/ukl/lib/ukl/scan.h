// ukl/scan.h: the device-wide exclusive scan of 32-bit values (decision 0400 §5), as two kernels the host plan
// launches (`rerail_gpu::library::scan_exclusive_u32`).
//
// Written for the weakest progress guarantee (Metal guarantees none between threadgroups): no threadgroup waits for
// another. A tile is a threadgroup's `UKL_SCAN_ITEMS` values a thread:
//
//   1. `ukl_scan_u32_tile_sums`: each tile's sum;
//   2. the tile sums scanned, by this same scan (the host recurses; each level is `UKL_SCAN_ITEMS * threads`
//      times shorter), into each tile's offset;
//   3. `ukl_scan_u32_tiles`: each tile scanned from its offset.
//
// The output has `n + 1` values: `out[n]` is the total (an offsets array, as a CSR's starts). Sums wrap at 2^32.

#pragma once
#include <ukl/group.h>

// Values a thread scans.
#define UKL_SCAN_ITEMS 4u

struct UklScanArgs {
    uint32_t n;
    // Whether `offsets` holds the tiles' offsets (else every tile starts at 0: one tile).
    uint32_t hasOffsets;
};
UKL_REFLECT_STRUCT(UklScanArgs, n, hasOffsets)

// A thread's values: `UKL_SCAN_ITEMS` in a row from its first; zero past `n`.
UKL_INLINE uint64_t ukl_scan_first(uint32_t tile, uint32_t lid, uint32_t lsz)
{
    return ((uint64_t)tile * lsz + lid) * UKL_SCAN_ITEMS;
}

DEFINE_KERNEL2(ukl, ukl_scan_u32_tile_sums,
               ARGS,     UklScanArgs, args,    ,
               ROBUFFER, uint32_t,    in,      "[n]",
               RWBUFFER, uint32_t,    sums,    "[tiles]",
               SHARED,   uint32_t,    scratch, UKL_TG_SCRATCH,
               TGID0,    uint32_t,    tile,    ,
               LID0,     uint32_t,    lid,     ,
               LSZ0,     uint32_t,    lsz,     )
{
    uint64_t first = ukl_scan_first(tile, lid, lsz);
    uint32_t s = 0u;
    for (uint32_t k = 0u; k < UKL_SCAN_ITEMS; k++)
        if (first + k < args.n)
            s += in[first + k];
    uint32_t t = ukl_tg_sum(s, lid, lsz, scratch);
    if (lid == 0u)
        sums[tile] = t;
}

DEFINE_KERNEL2(ukl, ukl_scan_u32_tiles,
               ARGS,     UklScanArgs, args,    ,
               ROBUFFER, uint32_t,    in,      "[n]",
               RWBUFFER, uint32_t,    out,     "[n + 1]",
               ROBUFFER, uint32_t,    offsets, "[tiles + 1]",
               SHARED,   uint32_t,    scratch, UKL_TG_SCRATCH,
               TGID0,    uint32_t,    tile,    ,
               LID0,     uint32_t,    lid,     ,
               LSZ0,     uint32_t,    lsz,     )
{
    uint64_t first = ukl_scan_first(tile, lid, lsz);
    uint32_t v[UKL_SCAN_ITEMS];
    uint32_t s = 0u;
    for (uint32_t k = 0u; k < UKL_SCAN_ITEMS; k++) {
        v[k] = first + k < args.n ? in[first + k] : 0u;
        s += v[k];
    }
    UklScan r = ukl_tg_exclusive_sum(s, lid, lsz, scratch);
    uint32_t base = args.hasOffsets != 0u ? offsets[tile] : 0u;
    uint32_t run = base + r.exclusive;
    for (uint32_t k = 0u; k < UKL_SCAN_ITEMS; k++) {
        if (first + k < args.n)
            out[first + k] = run;
        run += v[k];
    }
    // The total, from the last tile's last thread (tile 0 when there are no values).
    uint32_t tileValues = lsz * UKL_SCAN_ITEMS;
    uint32_t lastTile = args.n == 0u ? 0u : (args.n - 1u) / tileValues;
    if (tile == lastTile && lid == lsz - 1u)
        out[args.n] = base + r.total;
}
