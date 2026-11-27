// ukl/delta/atoms.h: an atoms block (`rerail_blocks::atoms`, decision 0401), found in and appended to where it lies.
// Its root is at the block header's root word; its four regions are self-relative spans (`RelSpan`) from it. A slot
// of its table is a hash's high half, then the id plus one (0: empty), probed linearly. One writer appends; a reader
// trusts only the count it reads.
#pragma once
#include <ukl/delta/text.h>

// A text's hash, as `rerail_blocks::hash_bytes` makes it: its bytes as little-endian words from a seed, the last
// zero-padded, then its length.
UKL_INLINE uint64_t atom_mix(uint64_t h, uint64_t v) {
    h = (h ^ v) * 0x9E3779B97F4A7C15ull;
    return h ^ (h >> 29);
}
UKL_INLINE uint64_t atom_hash(Text x) {
    uint64_t h = 0x243F6A8885A308D3ull, w = 0u;
    uint32_t at = x.at, u, n, len = 0u;
    while ((n = text_next(x, &at, &u)) != 0u && n != TEXT_BAD)
        for (uint32_t k = 0u; k < n; k++, len++) {
            w |= (uint64_t)(u >> (8u * k) & 0xFFu) << (8u * (len % 8u));
            if (len % 8u == 7u) { h = atom_mix(h, w); w = 0u; }
        }
    if (len % 8u != 0u) h = atom_mix(h, w);
    return atom_mix(h, len);
}

// An atoms block: its bytes, and its root's offset.
typedef struct Atoms {
    Blk b;
    uint64_t r;
} Atoms;

UKL_INLINE Atoms atoms_of(Blk b) { Atoms a = {b, U64(b, ATOMS_ROOT_WORD)}; return a; }
UKL_INLINE uint64_t atoms_word(Atoms a, uint32_t f) { return U64(a.b, a.r + f); }

// Where the span at root field `f` starts, its targets aligned to `align` (`Rel<T, R32>`: units of `align` from the
// field, aligned down).
UKL_INLINE uint64_t atoms_span(Atoms a, uint32_t f, uint32_t align) {
    uint64_t field = a.r + f;
    return (field & ~(uint64_t)(align - 1u)) + (uint64_t)((int64_t)I32(a.b, field) * (int64_t)align);
}
UKL_INLINE uint64_t atoms_end(Atoms a, uint64_t i) { return U64(a.b, atoms_span(a, ATOMS_ENDS_AT, 8u) + 8u * i); }

// Whether atom `i` (from its first) is the text's value.
UKL_INLINE bool atom_is(Atoms a, uint64_t i, Text x) {
    uint64_t s = atoms_span(a, ATOMS_STRINGS_AT, 1u), k = i == 0u ? 0u : atoms_end(a, i - 1u), end = atoms_end(a, i);
    uint32_t at = x.at, u, n;
    while ((n = text_next(x, &at, &u)) != 0u && n != TEXT_BAD)
        for (uint32_t j = 0u; j < n; j++, k++)
            if (k >= end || a.b[s + k] != (u >> (8u * j) & 0xFFu)) return false;
    return n == 0u && k == end;
}

// The text's id (its hash `h`), or NONE.
UKL_INLINE uint32_t atoms_find(Atoms a, Text x, uint64_t h) {
    uint64_t first = atoms_word(a, ATOMS_FIRST_AT), end = first + atoms_word(a, ATOMS_COUNT_AT), mask = atoms_word(a, ATOMS_MASK_AT);
    uint64_t table = atoms_span(a, ATOMS_TABLE_AT, 8u), tag = h & 0xFFFFFFFF00000000ull;
    for (uint64_t i = h & mask;; i = (i + 1u) & mask) {
        uint64_t v = U64(a.b, table + 8u * i);
        if (v == 0u) return NONE;
        uint32_t id = (uint32_t)v - 1u;
        if ((v & 0xFFFFFFFF00000000ull) == tag && id >= first && id < end && atom_is(a, id - first, x)) return id;
    }
}

// The text appended (its hash `h`), by the block's one writer: its id, or NONE when the block is full.
UKL_INLINE uint32_t atoms_append(Atoms a, Text x, uint64_t h) {
    uint64_t n = atoms_word(a, ATOMS_COUNT_AT), at = atoms_word(a, ATOMS_BYTES_LEN_AT), mask = atoms_word(a, ATOMS_MASK_AT);
    uint64_t len = text_len(x), placed = atoms_word(a, ATOMS_PLACED_AT);
    if (n >= atoms_word(a, ATOMS_CAP_AT) || at + len > atoms_word(a, ATOMS_BYTES_CAP_AT) || placed >= (mask + 1u) / 4u * 3u) return NONE;
    uint32_t id = (uint32_t)(atoms_word(a, ATOMS_FIRST_AT) + n);
    uint64_t s = atoms_span(a, ATOMS_STRINGS_AT, 1u) + at;
    uint32_t xa = x.at, u, m;
    while ((m = text_next(x, &xa, &u)) != 0u && m != TEXT_BAD)
        for (uint32_t j = 0u; j < m; j++) a.b[s++] = (uint8_t)(u >> (8u * j));
    U64(a.b, atoms_span(a, ATOMS_ENDS_AT, 8u) + 8u * n) = at + len;
    U64(a.b, atoms_span(a, ATOMS_HASHES_AT, 8u) + 8u * n) = h;
    U64(a.b, a.r + ATOMS_PLACED_AT) = placed + 1u;
    uint64_t table = atoms_span(a, ATOMS_TABLE_AT, 8u), i = h & mask;
    while (U64(a.b, table + 8u * i) != 0u) i = (i + 1u) & mask;
    U64(a.b, table + 8u * i) = (h & 0xFFFFFFFF00000000ull) | (uint64_t)(id + 1u);
    U64(a.b, a.r + ATOMS_BYTES_LEN_AT) = at + len;
    U64(a.b, a.r + ATOMS_COUNT_AT) = n + 1u;
    return id;
}

// The text's id, made when it has none (NONE: the block is full).
UKL_INLINE uint32_t atoms_put(Atoms a, Text x) {
    uint64_t h = atom_hash(x);
    uint32_t id = atoms_find(a, x, h);
    return id != NONE ? id : atoms_append(a, x, h);
}
