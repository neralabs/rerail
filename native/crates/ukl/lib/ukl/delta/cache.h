// ukl/delta/cache.h: an operator's cache (`rerail_delta_twins::cache`): its header, then its entries, each a header
// (its layer, its columns' count, four columns, its rows' and fences' counts), its rows' words and its fences. A sizes
// entry has no columns; its layer word is its document. An entry is found by the cache's index, an entry of its own
// (`CACHE_INDEX_COLS` columns): an open-addressed table of the entries' places by their keys' hash, made again twice
// the size at the cursor when one more key would fill more than half of it.
#pragma once
#include <ukl/delta/op.h>

// The entry after the one at `at`.
UKL_INLINE uint32_t cache_next(CBlk c, uint32_t at) {
    uint32_t n = U32(c, at + 24u), nf = U32(c, at + 28u);
    return at + CACHE_ENTRY_BYTES + ((4u * n + 7u) & ~7u) + 8u * nf;
}

// The entry at `at`'s column `k`.
UKL_INLINE uint32_t cache_col(CBlk c, uint32_t at, uint32_t k) { return (uint32_t)(*(CLIF_BLOCK_SPACE uint16_t*)(c + at + 8u + 2u * k)); }

// An entry's key's hash (`rerail_delta_twins::cache::key_hash`): its layer, its columns' count and its columns.
UKL_INLINE uint32_t cache_hash(uint32_t layer, uint32_t ncols, UKL_PRIVATE const uint32_t* cols) {
    uint64_t c = 0u;
    for (uint32_t i = 0u; i < ncols; i++) c |= (uint64_t)cols[i] << (16u * i);
    uint64_t h = ((uint64_t)layer ^ ((uint64_t)ncols << 40)) * 0x9E3779B97F4A7C15ull;
    h ^= c * 0xC2B2AE3D27D4EB4Full;
    h ^= h >> 29;
    return (uint32_t)(h >> 32);
}

// Whether the entry at `at` is a live one of key `(layer, cols)`.
UKL_INLINE bool cache_is_key(CBlk c, uint32_t at, uint32_t layer, uint32_t ncols, UKL_PRIVATE const uint32_t* cols) {
    if (U32(c, at) != layer || U32(c, at + 4u) != ncols) return false;
    for (uint32_t k = 0u; k < ncols; k++)
        if (cache_col(c, at, k) != cols[k]) return false;
    return true;
}

// The index's slots' count (0: none), and where they start.
UKL_INLINE uint32_t cache_slots(CBlk c) {
    uint32_t ix = (uint32_t)U64(c, CACHE_INDEX_AT);
    return ix == 0u ? 0u : U32(c, ix + 24u);
}
UKL_INLINE uint32_t cache_slots_at(CBlk c) { return (uint32_t)U64(c, CACHE_INDEX_AT) + CACHE_ENTRY_BYTES; }

// The entry of key `(layer, cols)`: where it lies, by the index, or NONE.
UKL_INLINE uint32_t cache_find(CBlk c, uint32_t layer, uint32_t ncols, UKL_PRIVATE const uint32_t* cols) {
    uint32_t n = cache_slots(c);
    if (n == 0u) return NONE;
    uint32_t s = cache_slots_at(c);
    for (uint32_t i = cache_hash(layer, ncols, cols) & (n - 1u);; i = (i + 1u) & (n - 1u)) {
        uint32_t at = U32(c, s + 4u * i);
        if (at == 0u) return NONE;
        if (cache_is_key(c, at, layer, ncols, cols)) return at;
    }
}

// Document `d`'s sizes entry: where it lies, or NONE.
UKL_INLINE uint32_t cache_sizes(CBlk c, uint32_t d) {
    uint32_t none[4] = {0u, 0u, 0u, 0u};
    return cache_find(c, d, 0u, none);
}

// Whether indexing one more key makes a new index (more than half its slots filled).
UKL_INLINE bool cache_grows(CBlk c) { return 2u * ((uint32_t)U64(c, CACHE_FILLED_AT) + 1u) > cache_slots(c); }

// The bytes a new index takes at the cursor before one more key is indexed (0: none is made).
UKL_INLINE uint32_t cache_index_room(CBlk c) {
    uint32_t n = 2u * cache_slots(c);
    return cache_grows(c) ? CACHE_ENTRY_BYTES + 4u * (n > CACHE_INDEX_LEAST ? n : CACHE_INDEX_LEAST) : 0u;
}

// The entry at `at` indexed by its key, in the index's first empty slot from its key's.
UKL_INLINE void cache_index_one(Blk b, uint32_t at) {
    uint32_t n = cache_slots(b), s = cache_slots_at(b), ncols = U32(b, at + 4u), cols[4];
    for (uint32_t k = 0u; k < 4u; k++) cols[k] = cache_col(b, at, k);
    uint32_t i = cache_hash(U32(b, at), ncols, cols) & (n - 1u);
    while (U32(b, s + 4u * i) != 0u) i = (i + 1u) & (n - 1u);
    U32(b, s + 4u * i) = at;
    U64(b, CACHE_FILLED_AT) = U64(b, CACHE_FILLED_AT) + 1u;
}

// Room in the index for one more key (`cache_index_room` bytes at the cursor, reached first): when it would be over
// half full, a new index twice the size at the cursor, of every live entry in the cache's order. Made before the entry
// it makes room for is written: an entry appended lies past it.
UKL_INLINE void cache_reindex(Blk b) {
    if (!cache_grows(b)) return;
    uint32_t n = 2u * cache_slots(b), at = CACHE_ENTRIES_AT + (uint32_t)U64(b, CACHE_USED_AT);
    n = n > CACHE_INDEX_LEAST ? n : CACHE_INDEX_LEAST;
    U32(b, at) = 0u;
    U32(b, at + 4u) = CACHE_INDEX_COLS;
    for (uint32_t k = 0u; k < 4u; k++) *(CLIF_BLOCK_SPACE uint16_t*)(b + at + 8u + 2u * k) = 0u;
    U32(b, at + 24u) = n;
    U32(b, at + 28u) = 0u;
    // An empty slot is 0: the slots are written before they are read.
    for (uint32_t k = 0u; k < n; k++) U32(b, at + CACHE_ENTRY_BYTES + 4u * k) = 0u;
    U64(b, CACHE_USED_AT) = at + CACHE_ENTRY_BYTES + 4u * n - CACHE_ENTRIES_AT;
    U64(b, CACHE_INDEX_AT) = at;
    U64(b, CACHE_FILLED_AT) = 0u;
    for (uint32_t e = CACHE_ENTRIES_AT; e < at; e = cache_next(b, e))
        if (U32(b, e + 4u) <= 4u && U32(b, e) != CACHE_DEAD) cache_index_one(b, e);
}
