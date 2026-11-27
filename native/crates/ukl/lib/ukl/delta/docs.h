// ukl/delta/docs.h: a docs block (`rerail_delta_twins::k0`): the documents a pass reads, each its module, its source's,
// tape's and symbols' blocks and where its source and tape lie in theirs; and the first document a blob of which no
// blobs' row names (NO_DOC: none). And the blobs' rows that name them: `(name's four words, block, at, len)`.
#pragma once
#include <ukl/delta/trace.h>

#define DOC(d, k, w) U64(d, DOCS_AT + 8u * (DOC_WORDS * (k) + (w)))

UKL_INLINE uint32_t docs_len(CBlk d) { return (uint32_t)U64(d, DOCS_N_AT); }
UKL_INLINE int64_t docs_module(CBlk d, uint32_t k) { return (int64_t)DOC(d, k, 0u); }

// The first `n` documents sealed.
UKL_INLINE void docs_seal(Blk d, uint32_t n) {
    U64(d, DOCS_LEN_AT) = DOCS_AT - BODY_AT + 8u * DOC_WORDS * n;
    U64(d, DOCS_N_AT) = n;
}

// A blob's place: its block, and where it lies there.
typedef struct Blob {
    uint32_t block;
    uint64_t at, len;
} Blob;

// The blob named `w` (four words) by the blobs' rows at `blobs`: false when no live row names it.
UKL_INLINE bool blob_find(CBlk t, uint32_t blobs, UKL_PRIVATE const int64_t* w, UKL_PRIVATE Blob* b) {
    for (uint32_t id = 0u, end = state_end(t, blobs); id < end; id++) {
        if (!state_live(t, blobs, id)) continue;
        bool same = true;
        for (uint32_t c = 0u; c < 4u && same; c++) same = state_get(t, blobs, id, c) == w[c];
        if (!same) continue;
        b->block = (uint32_t)state_get(t, blobs, id, 4u);
        b->at = (uint64_t)state_get(t, blobs, id, 5u);
        b->len = (uint64_t)state_get(t, blobs, id, 6u);
        return true;
    }
    return false;
}
