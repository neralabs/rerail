// ukl/delta/cache.h: an operator's cache (`rerail_delta_twins::cache`): its header, then its entries, each a header
// (its layer, its columns' count, four columns, its rows' and fences' counts), its rows' words and its fences. A sizes
// entry has no columns; its layer word is its document.
#pragma once
#include <ukl/delta/op.h>

// The entry after the one at `at`.
UKL_INLINE uint32_t cache_next(CBlk c, uint32_t at) {
    uint32_t n = U32(c, at + 24u), nf = U32(c, at + 28u);
    return at + CACHE_ENTRY_BYTES + ((4u * n + 7u) & ~7u) + 8u * nf;
}

// Document `d`'s sizes entry: where it lies, or NONE.
UKL_INLINE uint32_t cache_sizes(CBlk c, uint32_t d) {
    uint32_t end = CACHE_ENTRIES_AT + (uint32_t)U64(c, CACHE_USED_AT);
    for (uint32_t at = CACHE_ENTRIES_AT; at < end; at = cache_next(c, at))
        if (U32(c, at + 4u) == 0u && U32(c, at) == d) return at;
    return NONE;
}
