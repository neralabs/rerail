// ukl/scan.h: the device-wide exclusive scan of 32-bit values (decision 0440 §5), as two kernels the host plan
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
// With `popcount`, each value counts as its set bits: the scan of a bitset's words gives each word's first rank.
// `out` may be `in`: each thread writes only the values it read, after reading them.
//
// A scan sized on the device (`level` set) takes each level's count from `sizes`, which `ukl_scan_u32_size` writes
// from a count the work before it made, and is launched on the grids it writes there.

#pragma once
#include <ukl/group.h>

// Values a thread scans, and a tile's (`UKL_SCAN_THREADS` threads).
#define UKL_SCAN_ITEMS 4u
#define UKL_SCAN_THREADS 256u
#define UKL_SCAN_TILE (UKL_SCAN_ITEMS * UKL_SCAN_THREADS)
// A sized scan's levels at most, and its `sizes` words: each level's count (0..8), its tiles as a grid
// (8 + 3k..), and a grid over the `n + 1` outputs for the launches around it (32..35).
#define UKL_SCAN_LEVELS 8u
#define UKL_SCAN_SIZES 36u

typedef struct UklScanArgs {
    uint32_t n;
    // Whether `offsets` holds the tiles' offsets (else every tile starts at 0: one tile).
    uint32_t hasOffsets;
    // Whether each value counts as its set bits.
    uint32_t popcount;
    // 0, or a sized scan's level plus one: its count is `sizes[level - 1]`, not `n`.
    uint32_t level;
} UklScanArgs;
UKL_REFLECT_STRUCT(UklScanArgs, n, hasOffsets, popcount, level)

// The arguments with the level's count.
UKL_INLINE UklScanArgs ukl_scan_sized(UklScanArgs args, UKL_GLOBAL const uint32_t* sizes)
{
    if (args.level != 0u)
        args.n = sizes[args.level - 1u];
    return args;
}

typedef struct UklScanSizeArgs {
    // The count is `min(counts[slot], cap)`.
    uint32_t slot;
    uint32_t cap;
    uint32_t levels;
    // The threads a group of the grid over the outputs.
    uint32_t threads;
} UklScanSizeArgs;
UKL_REFLECT_STRUCT(UklScanSizeArgs, slot, cap, levels, threads)

// A sized scan's `sizes` (`UKL_SCAN_SIZES` words), from the count the work before it wrote. One thread.
DEFINE_KERNEL2(ukl, ukl_scan_u32_size,
               ARGS,     UklScanSizeArgs, args,   ,
               ROBUFFER, uint32_t,        counts, "[slot + 1]",
               RWBUFFER, uint32_t,        sizes,  "[UKL_SCAN_SIZES]",
               GID0,     uint32_t,        t,      )
{
    if (t != 0u)
        return;
    uint32_t n = ukl_min(counts[args.slot], args.cap);
    sizes[32] = n / args.threads + 1u;
    sizes[33] = 1u;
    sizes[34] = 1u;
    for (uint32_t k = 0u; k < args.levels && k < UKL_SCAN_LEVELS; k++) {
        uint32_t g = ukl_max((n + UKL_SCAN_TILE - 1u) / UKL_SCAN_TILE, 1u);
        sizes[k] = n;
        sizes[8u + 3u * k] = g;
        sizes[9u + 3u * k] = 1u;
        sizes[10u + 3u * k] = 1u;
        n = g;
    }
}

// A thread's values: `UKL_SCAN_ITEMS` in a row from its first; zero past `n`.
UKL_INLINE uint64_t ukl_scan_first(uint32_t tile, uint32_t lid, uint32_t lsz)
{
    return ((uint64_t)tile * lsz + lid) * UKL_SCAN_ITEMS;
}

// The tile passes over the device's memory (`ukl/body/scan_tile.h`).
#define UKL_SCAN_NAME(f) f##_global
#define UKL_SCAN_SPACE UKL_GLOBAL
#include <ukl/body/scan_tile.h>

DEFINE_KERNEL2(ukl, ukl_scan_u32_tile_sums,
               ARGS,     UklScanArgs, args,    ,
               ROBUFFER, uint32_t,    in,      "[n]",
               RWBUFFER, uint32_t,    sums,    "[tiles]",
               ROBUFFER, uint32_t,    sizes,   "[UKL_SCAN_SIZES]",
               SHARED,   uint32_t,    scratch, UKL_TG_SCRATCH,
               TGID0,    uint32_t,    tile,    ,
               LID0,     uint32_t,    lid,     ,
               LSZ0,     uint32_t,    lsz,     )
{
    ukl_scan_tile_sum_global(in, sums, ukl_scan_sized(args, sizes), scratch, tile, lid, lsz __UKL_ARGS);
}

DEFINE_KERNEL2(ukl, ukl_scan_u32_tiles,
               ARGS,     UklScanArgs, args,    ,
               ROBUFFER, uint32_t,    in,      "[n]",
               RWBUFFER, uint32_t,    out,     "[n + 1]",
               ROBUFFER, uint32_t,    offsets, "[tiles + 1]",
               ROBUFFER, uint32_t,    sizes,   "[UKL_SCAN_SIZES]",
               SHARED,   uint32_t,    scratch, UKL_TG_SCRATCH,
               TGID0,    uint32_t,    tile,    ,
               LID0,     uint32_t,    lid,     ,
               LSZ0,     uint32_t,    lsz,     )
{
    ukl_scan_tile_global(in, out, offsets, ukl_scan_sized(args, sizes), scratch, tile, lid, lsz __UKL_ARGS);
}
