// ukl/delta/relw.h: a relation written where it lies, in the block of slot `k` (`rerail_blocks::relation`'s
// `try_create_cols`, `try_continue_far_cols`, `Rows::try_reserve`, `layer_write`'s `open`, `stat`, `index`, `seal`):
// the same descriptor, column directories, chunks and hashed index, so every reader reads it as the host's writer
// left it. Its room is claimed from the block's front (`claim`); a claim past the capacity returns false, and the
// launch runs again over the grown block (decision 0686 D6). Values are written by the reads written once in CLIF
// (`clif_rows_put`, decision 0310).
#pragma once
#include <ukl/delta/op.h>
#include <ukl/reads/relation.h>

// A span (`flat::RelSpan<T>`) at byte `f` of block `b`, set to `n` items at byte `to`, in units of `align` from its
// field aligned down; none when `n` is 0.
UKL_INLINE void span_set(Blk b, uint32_t f, uint32_t to, uint32_t n, uint32_t align) {
    int64_t base = (int64_t)(f & ~(align - 1u));
    I32(b, f) = n == 0u ? 0 : (int32_t)(((int64_t)to - base) / (int64_t)align);
    U32(b, f + 4u) = n;
}
// Where a span's items start, and how many.
UKL_INLINE uint32_t span_at(CBlk b, uint32_t f, uint32_t align) { return (uint32_t)((int64_t)(f & ~(align - 1u)) + (int64_t)I32(b, f) * align); }
UKL_INLINE uint32_t span_len(CBlk b, uint32_t f) { return U32(b, f + 4u); }

// The first chunk's rows a relation of `n` rows takes (`layer::one_chunk`): their log.
UKL_INLINE uint32_t one_chunk(uint32_t n) {
    uint32_t lg = 0u;
    while ((1u << lg) < (n > 1u ? n : 1u)) lg++;
    return lg + 2u > ROWS_MIN_LOG ? lg + 2u : ROWS_MIN_LOG;
}

// The rows chunk `s` holds, and the rows the first `n` chunks hold (`rows::chunk_rows`, `rows::capacity`).
UKL_INLINE uint64_t chunk_rows(uint32_t s, uint32_t min_log) { return 1ull << (min_log + s / 4u - 2u); }
UKL_INLINE uint64_t chunks_hold(uint32_t n, uint32_t min_log) {
    uint32_t k = n / 4u;
    return (((1ull << k) - 1u) << min_log) + (uint64_t)(n % 4u) * chunk_rows(n, min_log);
}

// A row set (`rows::Rows`) at byte `r` of the block: its columns' directories at `dirs`, of widths `widths`
// (`rows::I32`: a signed 32-bit column), its first chunk `2^min_log` rows (`Rows::init_in`).
UKL_INLINE void rows_init(Blk b, uint32_t r, uint32_t dirs, UKL_PRIVATE const uint32_t* widths, uint32_t n, uint32_t min_log) {
    for (uint32_t c = 0u; c < n; c++) {
        U32(b, dirs + c * COLDIR_BYTES + COLDIR_WIDTH_AT) = widths[c] & 0xFFu;
        I32(b, dirs + c * COLDIR_BYTES + COLDIR_RUNS_AT) = widths[c] == ROWS_I32 ? (int32_t)ROWS_SIGNED : 0;
    }
    U32(b, r + ROWS_MIN_LOG_AT) = min_log;
    U32(b, r + ROWS_NCOLS_AT) = n;
    span_set(b, r + ROWS_COLS_AT, dirs, n, 8u);
}

// A relation of `arity` columns of widths `widths`, sorted on `sort` (NONE: none), its first chunk for `n` rows, made in
// slot `k`'s block (`try_create_cols`): its descriptor's offset, or NONE.
UKL_INLINE uint32_t rel_create(Blk b, UKL_GLOBAL uint32_t* meter, uint32_t k, UKL_PRIVATE const uint32_t* widths, uint32_t arity, uint32_t sort, uint32_t n) {
    uint32_t bytes = REL_DIRS_AT + (arity + 1u) * COLDIR_BYTES;
    uint32_t at = claim(meter, k, bytes, 64u);
    if (at == NONE) return NONE;
    // Its clear state written over the claim whole, then its fields.
    for (uint32_t w = 0u; w < bytes; w += 8u) U64(b, at + w) = 0u;
    U32(b, at + REL_ARITY_AT) = arity;
    U32(b, at + REL_BASE_BLOCK_AT) = NONE;
    *(CLIF_BLOCK_SPACE uint16_t*)(b + at + REL_OKEY_AT) = (uint16_t)(sort == NONE ? 0u : sort);
    U32(b, at + REL_NOKEY_AT) = sort == NONE ? 0u : 1u;
    rows_init(b, at + REL_ROWS_AT, at + REL_DIRS_AT, widths, arity, one_chunk(n));
    const uint32_t killed[1] = {4u};
    rows_init(b, at + REL_KILLED_AT, at + REL_DIRS_AT + arity * COLDIR_BYTES, killed, 1u, 6u);
    return at;
}

// The relation at `at` made to continue the frozen relation at byte `base_at` of block `below`, whose rows end at
// `end` (`try_continue_far_cols`): its ids start at `end`.
UKL_INLINE void rel_continue_far(Blk b, uint32_t at, uint32_t below, uint32_t base_at, uint32_t end) {
    U32(b, at + REL_BASE_ROWS_AT) = end;
    U32(b, at + REL_BASE_BLOCK_AT) = below;
    I64(b, at + REL_BASE_OFF_AT) = (int64_t)base_at;
    U32(b, at + REL_FLAGS_AT) |= REL_DELTA | REL_CONTINUES | REL_FAR_BASE;
}

// Room for the relation's first `n` own rows: each column's chunks made until they hold them (`Rows::try_reserve`),
// then the rows claimed. False when the block has no room.
UKL_INLINE bool rel_claim_rows(Blk b, UKL_GLOBAL uint32_t* meter, uint32_t k, uint32_t at, uint32_t n) {
    uint32_t r = at + REL_ROWS_AT, min_log = U32(b, r + ROWS_MIN_LOG_AT), ncols = U32(b, r + ROWS_NCOLS_AT);
    uint32_t dirs = span_at(b, r + ROWS_COLS_AT, 8u);
    for (uint32_t s = U32(b, r + ROWS_NCHUNKS_AT); chunks_hold(s, min_log) < n; s++) {
        uint32_t rows = (uint32_t)chunk_rows(s, min_log);
        for (uint32_t c = 0u; c < ncols; c++) {
            uint32_t d = dirs + c * COLDIR_BYTES, bytes = rows * U32(b, d + COLDIR_WIDTH_AT);
            uint32_t chunk = claim(meter, k, bytes, bytes >= BLOCK_PAGE ? BLOCK_PAGE : 64u);
            if (chunk == NONE) return false;
            I64(b, d + COLDIR_CHUNK_AT + 8u * s) = (int64_t)chunk - (int64_t)d;
        }
        U32(b, r + ROWS_NCHUNKS_AT) = s + 1u;
        U32(b, r + ROWS_RESERVED_AT) = (uint32_t)chunks_hold(s + 1u, min_log);
    }
    U32(b, r + ROWS_LEN_AT) += n;
    return true;
}

// Own row `i`'s value `v` in column `c` (its `i`th from the relation's first own row).
UKL_INLINE void rel_put(Blk b, uint32_t at, uint32_t i, uint32_t c, int64_t v) { clif_rows_put(b, at + REL_ROWS_AT, i, c, (uint64_t)v); }
UKL_INLINE int64_t rel_own(CBlk b, uint32_t at, uint32_t i, uint32_t c) { return (int64_t)clif_rows_get(b, at + REL_ROWS_AT, i, c); }
UKL_INLINE uint32_t rel_own_len(CBlk b, uint32_t at) { return U32(b, at + REL_ROWS_AT + ROWS_LEN_AT); }

// The freeze's directories (`layer_write::open`): a range a column, the index directory (room for `keys` hashed indexes
// and the orders added after), and a sketch a column when `sketched`. False when the block has no room.
UKL_INLINE bool rel_open(Blk b, UKL_GLOBAL uint32_t* meter, uint32_t k, uint32_t at, uint32_t keys, bool sketched) {
    uint32_t a = U32(b, at + REL_ARITY_AT), n = a > 1u ? a : 1u, room = keys > REL_ADDED_INDEXES ? keys : REL_ADDED_INDEXES;
    uint32_t sp = claim(meter, k, 16u * n, 8u), ix = claim(meter, k, INDEX_BYTES * room, 8u);
    if (sp == NONE || ix == NONE) return false;
    for (uint32_t c = 0u; c < a; c++) {
        I64(b, sp + 16u * c) = 0x7FFFFFFFFFFFFFFFll;
        I64(b, sp + 16u * c + 8u) = -0x7FFFFFFFFFFFFFFFll - 1;
    }
    for (uint32_t w = 0u; w < INDEX_BYTES * room; w += 8u) U64(b, ix + w) = 0u;
    span_set(b, at + REL_STATS_AT, sp, a, 8u);
    span_set(b, at + REL_INDEXES_AT, ix, room, 8u);
    if (sketched) {
        uint32_t sk = claim(meter, k, 8u * n, 8u);
        if (sk == NONE) return false;
        for (uint32_t c = 0u; c < n; c++) U64(b, sk + 8u * c) = 0u;
        span_set(b, at + REL_SKETCHES_AT, sk, 8u * a, 1u);
    }
    return true;
}

// Column `c`'s range over the relation's own rows, into its statistics (`layer_write::stat`).
UKL_INLINE void rel_stat(Blk b, uint32_t at, uint32_t c) {
    int64_t lo = 0x7FFFFFFFFFFFFFFFll, hi = -0x7FFFFFFFFFFFFFFFll - 1;
    for (uint32_t i = 0u, n = rel_own_len(b, at); i < n; i++) {
        int64_t v = rel_own(b, at, i, c);
        lo = v < lo ? v : lo;
        hi = v > hi ? v : hi;
    }
    uint32_t sp = span_at(b, at + REL_STATS_AT, 8u);
    I64(b, sp + 16u * c) = lo;
    I64(b, sp + 16u * c + 8u) = hi;
}

// A key's hash (`rerail_blocks::key_hash`): its values mixed in order into the seed, as every probe hashes it.
// Own row `i`'s hash on the `nk` columns `key` (`hash::key_hash`), and whether rows `i` and `j` agree on them.
UKL_INLINE uint64_t own_hash(CBlk b, uint32_t at, uint32_t i, UKL_PRIVATE const uint32_t* key, uint32_t nk) {
    uint64_t h = KEY_SEED;
    for (uint32_t t = 0u; t < nk; t++) h = clif_rel_key_mix(b, 0u, h, (uint64_t)rel_own(b, at, i, key[t]));
    return h;
}
UKL_INLINE bool own_same(CBlk b, uint32_t at, uint32_t i, uint32_t j, UKL_PRIVATE const uint32_t* key, uint32_t nk) {
    for (uint32_t t = 0u; t < nk; t++)
        if (rel_own(b, at, i, key[t]) != rel_own(b, at, j, key[t])) return false;
    return true;
}

// Hashed index `x` on the `nk` columns `key`, into its directory's entry (`layer_write::index`, `build_one_index`): a
// key of the one sort column an ordered entry; else its rows in buckets by their key's hash, in id order within each,
// and each place's mark (decision 0671). `2^k` buckets for its rows (the CPU's for its distinct keys, at most as
// many: a reader takes the mask the entry holds). False when the block has no room.
UKL_INLINE bool rel_index(Blk b, UKL_GLOBAL uint32_t* meter, uint32_t k, uint32_t at, uint32_t x, UKL_PRIVATE const uint32_t* key, uint32_t nk) {
    uint32_t live = rel_own_len(b, at), br = U32(b, at + REL_BASE_ROWS_AT);
    uint32_t ix = span_at(b, at + REL_INDEXES_AT, 8u) + x * INDEX_BYTES;
    U32(b, ix + INDEX_NCOLS_AT) = nk;
    for (uint32_t t = 0u; t < nk; t++) *(CLIF_BLOCK_SPACE uint16_t*)(b + ix + INDEX_KEY_AT + 2u * t) = (uint16_t)key[t];
    if (nk == 1u && U32(b, at + REL_NOKEY_AT) == 1u && *(CLIF_BLOCK_SPACE uint16_t*)(b + at + REL_OKEY_AT) == key[0]) {
        U32(b, ix + INDEX_ORDERED_AT) = 1u;
        return true;
    }
    uint32_t cap = 1u;
    while (cap < live) cap *= 2u;
    uint32_t offs = claim(meter, k, 4u * (cap + 1u), 8u), rows = claim(meter, k, 8u * (live > 1u ? live : 1u), 8u);
    if (offs == NONE || rows == NONE) return false;
    uint32_t marks = rows + 4u * live;
    for (uint32_t q = 0u; q <= cap; q++) U32(b, offs + 4u * q) = 0u;
    for (uint32_t j = 0u; j < live; j++) U32(b, offs + 4u * (((uint32_t)own_hash(b, at, j, key, nk) & (cap - 1u)) + 1u)) += 1u;
    for (uint32_t q = 0u; q < cap; q++) U32(b, offs + 4u * (q + 1u)) += U32(b, offs + 4u * q);
    // Each row placed after its bucket's rows before it, in id order: its bucket's start, and the rows of it before it.
    for (uint32_t j = 0u; j < live; j++) {
        uint64_t h = own_hash(b, at, j, key, nk);
        uint32_t bucket = (uint32_t)h & (cap - 1u), place = U32(b, offs + 4u * bucket);
        for (uint32_t q = 0u; q < j; q++) place += ((uint32_t)own_hash(b, at, q, key, nk) & (cap - 1u)) == bucket ? 1u : 0u;
        U32(b, rows + 4u * place) = br + j;
        U32(b, marks + 4u * place) = (uint32_t)(h >> 32) & ~INDEX_MARK_SAME;
    }
    // A place's mark says whether its bucket's previous place of its tag, among the `MARK_REACH` before it, holds its key.
    for (uint32_t q = 0u; q < cap; q++) {
        uint32_t lo = U32(b, offs + 4u * q), hi = U32(b, offs + 4u * (q + 1u));
        for (uint32_t p = lo; p < hi; p++) {
            uint32_t tag = U32(b, marks + 4u * p) & ~INDEX_MARK_SAME;
            for (uint32_t u = p; u > lo && u + INDEX_MARK_REACH > p; u--) {
                if ((U32(b, marks + 4u * (u - 1u)) & ~INDEX_MARK_SAME) != tag) continue;
                if (own_same(b, at, U32(b, rows + 4u * (u - 1u)) - br, U32(b, rows + 4u * p) - br, key, nk)) U32(b, marks + 4u * p) |= INDEX_MARK_SAME;
                break;
            }
        }
    }
    U64(b, ix + INDEX_MASK_AT) = (uint64_t)(cap - 1u);
    span_set(b, ix + INDEX_OFFS_AT, offs, cap + 1u, 4u);
    span_set(b, ix + INDEX_ROWS_AT, rows, live, 4u);
    span_set(b, ix + INDEX_MARKS_AT, marks, live, 4u);
    return true;
}

// The relation frozen (`layer_write::seal`): sorted on `sort`, `keys` hashed indexes, its own rows live.
UKL_INLINE void rel_seal(Blk b, uint32_t at, uint32_t sort, uint32_t keys) {
    *(CLIF_BLOCK_SPACE uint16_t*)(b + at + REL_OKEY_AT) = (uint16_t)(sort == NONE ? 0u : sort);
    U32(b, at + REL_NOKEY_AT) = sort == NONE ? 0u : 1u;
    U32(b, at + REL_NINDEX_AT) = keys;
    U32(b, at + REL_LIVE_AT) = rel_own_len(b, at);
    U32(b, at + REL_FLAGS_AT) |= REL_FROZEN;
}
