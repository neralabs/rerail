// ukl/body/scan_tile.h: ukl/scan.h's two passes over one tile, in one address space (decision 0440 §3.5: a space in
// C is a parameter of an included body, as ukl/body/radix_tile.h's key type). Included once per space, with, defined
// first:
// - `UKL_SCAN_NAME(f)`: the instance's name of each function `f` (`f##_global`, ...);
// - `UKL_SCAN_SPACE`: the address space of the values, the sums and the offsets (`UKL_GLOBAL`; coherent where other
//   threadgroups of the dispatch wrote them, as a join program's phases do, decision 0603).
// They are undefined at the end. Not `#pragma once`.

#if !defined(UKL_SCAN_NAME) || !defined(UKL_SCAN_SPACE)
#error "ukl/body/scan_tile.h needs UKL_SCAN_NAME and UKL_SCAN_SPACE (ukl/scan.h)"
#endif

UKL_INLINE uint32_t UKL_SCAN_NAME(ukl_scan_load)(UKL_SCAN_SPACE const uint32_t* in, uint64_t i, UklScanArgs args)
{
    if (i >= args.n)
        return 0u;
    uint32_t v = in[i];
    return args.popcount != 0u ? ukl_popcount32(v) : v;
}

// Tile `tile`'s sum into `sums[tile]`.
UKL_INLINE void UKL_SCAN_NAME(ukl_scan_tile_sum)(UKL_SCAN_SPACE const uint32_t* in, UKL_SCAN_SPACE uint32_t* sums, UklScanArgs a, UKL_LOCAL uint32_t* scratch,
                                                 uint32_t tile, uint32_t lid, uint32_t lsz __UKL_PARAMS)
{
    uint64_t first = ukl_scan_first(tile, lid, lsz);
    uint32_t s = 0u;
    for (uint32_t k = 0u; k < UKL_SCAN_ITEMS; k++)
        s += UKL_SCAN_NAME(ukl_scan_load)(in, first + k, a);
    uint32_t t = ukl_tg_sum(s, lid, lsz, scratch __UKL_ARGS);
    if (lid == 0u)
        sums[tile] = t;
}

// Tile `tile` scanned from its offset (`offsets[tile]` where `a.hasOffsets`, else 0) into `out` (which may be `in`:
// each thread writes only the values it read, after reading them); the last tile's last thread writes the total at
// `out[a.n]`.
UKL_INLINE void UKL_SCAN_NAME(ukl_scan_tile)(UKL_SCAN_SPACE const uint32_t* in, UKL_SCAN_SPACE uint32_t* out, UKL_SCAN_SPACE const uint32_t* offsets, UklScanArgs a,
                                             UKL_LOCAL uint32_t* scratch, uint32_t tile, uint32_t lid, uint32_t lsz __UKL_PARAMS)
{
    uint64_t first = ukl_scan_first(tile, lid, lsz);
    uint32_t v[UKL_SCAN_ITEMS];
    uint32_t s = 0u;
    for (uint32_t k = 0u; k < UKL_SCAN_ITEMS; k++) {
        v[k] = UKL_SCAN_NAME(ukl_scan_load)(in, first + k, a);
        s += v[k];
    }
    UklScan r = ukl_tg_exclusive_sum(s, lid, lsz, scratch __UKL_ARGS);
    uint32_t base = a.hasOffsets != 0u ? offsets[tile] : 0u;
    uint32_t run = base + r.exclusive;
    for (uint32_t k = 0u; k < UKL_SCAN_ITEMS; k++) {
        if (first + k < a.n)
            out[first + k] = run;
        run += v[k];
    }
    // The total, from the last tile's last thread (tile 0 when there are no values).
    uint32_t tileValues = lsz * UKL_SCAN_ITEMS;
    uint32_t lastTile = a.n == 0u ? 0u : (a.n - 1u) / tileValues;
    if (tile == lastTile && lid == lsz - 1u)
        out[a.n] = base + r.total;
}

#undef UKL_SCAN_NAME
#undef UKL_SCAN_SPACE
