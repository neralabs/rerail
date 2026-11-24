// ukl/reads/aux_index.h: a tape's auxiliary index read in place (decision 0656): the reads of `rerail_tape_format::aux_index`,
// the same bytes read the same way, written by hand. `rerail-ukl-vec`'s `aux_index_tests` hold every read here equal
// to the Rust one on every node, leaf, term and code.
//
// A read takes the auxiliary index's first byte (`s`, 16-byte aligned) and its own parameters. An auxiliary index without the
// section a read needs answers `UKL_AUX_NONE` (a count: `UKL_AUX_NONE`; a leaf's rank: `UKL_AUX_NONE`). The layout is
// the Rust module's: a header, a directory of (kind u16, version u16, params u32, offset u32, length u32), then each
// section, 16-byte aligned, after four u32s of its own.
//
// The reads that walk the tape too (a child's place, a parent's i-th child) are `<ukl/reads/aux_index_tape.h>`.

#pragma once
#include <ukl/memory.h>

#define UKL_AUX_NONE 0xffffffffu
#define UKL_AUX_MAGIC 0x31584152u
#define UKL_AUX_VERSION 1u
#define UKL_AUX_BLOCK 256u
#define UKL_AUX_CHUNK 256u
#define UKL_AUX_ESCAPE 255u
#define UKL_AUX_K 8u
// The sections' kinds, as `aux_index::Kind`.
#define UKL_AUX_LEAVES 1u
#define UKL_AUX_NAMES 2u
#define UKL_AUX_VOCAB 3u
#define UKL_AUX_SIBLINGS 4u
#define UKL_AUX_COUNTS 5u
#define UKL_AUX_RARE 6u

#define UKL_AUX_ARGS UKL_GLOBAL const uint8_t* s

UKL_INLINE uint32_t ukl_aux_u32(UKL_AUX_ARGS, uint32_t at) { return ukl_ld_u32(s, at); }
UKL_INLINE uint64_t ukl_aux_u64(UKL_AUX_ARGS, uint32_t at) { return ukl_ld_u64(s, at); }

UKL_INLINE uint32_t ukl_aux_nodes(UKL_AUX_ARGS) { return ukl_aux_u32(s, 8u); }
UKL_INLINE uint32_t ukl_aux_leaves(UKL_AUX_ARGS) { return ukl_aux_u32(s, 12u); }
UKL_INLINE uint32_t ukl_aux_terms(UKL_AUX_ARGS) { return ukl_aux_u32(s, 20u); }

// Section `kind`'s offset, or 0 where the auxiliary index has none (or is none, or of another version).
UKL_INLINE uint32_t ukl_aux_section(UKL_AUX_ARGS, uint32_t kind)
{
    if (ukl_aux_u32(s, 0u) != UKL_AUX_MAGIC || (ukl_aux_u32(s, 4u) & 0xffffu) != UKL_AUX_VERSION)
        return 0u;
    uint32_t count = ukl_aux_u32(s, 4u) >> 16;
    for (uint32_t e = 0u; e < count; e++) {
        uint32_t d = 32u + 16u * e;
        if ((ukl_aux_u32(s, d) & 0xffffu) == kind)
            return (ukl_aux_u32(s, d) >> 16) == 1u ? ukl_aux_u32(s, d + 8u) : 0u;
    }
    return 0u;
}

// ---- LEAVES ----

// `node`'s rank among the leaves, or `UKL_AUX_NONE` (an inner node, or no LEAVES).
UKL_INLINE uint32_t ukl_aux_leaf_rank(UKL_AUX_ARGS, uint32_t node)
{
    uint32_t sec = ukl_aux_section(s, UKL_AUX_LEAVES);
    if (sec == 0u || node >= ukl_aux_nodes(s))
        return UKL_AUX_NONE;
    uint32_t b = node / UKL_AUX_BLOCK, bit = node % UKL_AUX_BLOCK;
    uint32_t words = sec + 16u + 32u * b;
    uint64_t w = ukl_aux_u64(s, words + 8u * (bit / 64u));
    if (((w >> (bit % 64u)) & 1u) == 0u)
        return UKL_AUX_NONE;
    uint32_t r = ukl_aux_u32(s, sec + ukl_aux_u32(s, sec + 4u) + 4u * b);
    for (uint32_t k = 0u; k < bit / 64u; k++)
        r += ukl_popcount64(ukl_aux_u64(s, words + 8u * k));
    return r + ukl_rank64(w, bit % 64u);
}

// The node of leaf `rank`, or `UKL_AUX_NONE`.
UKL_INLINE uint32_t ukl_aux_leaf_node(UKL_AUX_ARGS, uint32_t rank)
{
    uint32_t sec = ukl_aux_section(s, UKL_AUX_LEAVES);
    if (sec == 0u || rank >= ukl_aux_leaves(s))
        return UKL_AUX_NONE;
    uint32_t blocks = ukl_aux_u32(s, sec), bases = sec + ukl_aux_u32(s, sec + 4u);
    uint32_t lo = 0u, hi = blocks;
    while (hi - lo > 1u) {
        uint32_t mid = (lo + hi) / 2u;
        if (ukl_aux_u32(s, bases + 4u * mid) <= rank)
            lo = mid;
        else
            hi = mid;
    }
    uint32_t r = rank - ukl_aux_u32(s, bases + 4u * lo);
    for (uint32_t k = 0u; k < 4u; k++) {
        uint64_t w = ukl_aux_u64(s, sec + 16u + 32u * lo + 8u * k);
        uint32_t c = ukl_popcount64(w);
        if (r < c)
            return lo * UKL_AUX_BLOCK + 64u * k + ukl_select64(w, r);
        r -= c;
    }
    return UKL_AUX_NONE;
}

// ---- NAMES and VOCAB ----

// Leaf `leaf`'s term, or `UKL_AUX_NONE`.
UKL_INLINE uint32_t ukl_aux_term_of_leaf(UKL_AUX_ARGS, uint32_t leaf)
{
    uint32_t sec = ukl_aux_section(s, UKL_AUX_NAMES);
    if (sec == 0u || leaf >= ukl_aux_leaves(s))
        return UKL_AUX_NONE;
    uint32_t x = ukl_ld_u8(s, sec + 16u + leaf);
    if (x != UKL_AUX_ESCAPE)
        return x;
    uint32_t bases = sec + ukl_aux_u32(s, sec + 4u), escapes = sec + ukl_aux_u32(s, sec + 8u);
    uint32_t c = leaf / UKL_AUX_CHUNK;
    uint32_t lo = ukl_aux_u32(s, bases + 4u * c), hi = ukl_aux_u32(s, bases + 4u * c + 4u);
    while (lo < hi) {
        uint32_t mid = (lo + hi) / 2u;
        uint32_t l = ukl_aux_u32(s, escapes + 8u * mid);
        if (l == leaf)
            return ukl_aux_u32(s, escapes + 8u * mid + 4u);
        if (l < leaf)
            lo = mid + 1u;
        else
            hi = mid;
    }
    return UKL_AUX_NONE;
}

// `node`'s term (a leaf's), or `UKL_AUX_NONE`.
UKL_INLINE uint32_t ukl_aux_term_of_node(UKL_AUX_ARGS, uint32_t node)
{
    uint32_t l = ukl_aux_leaf_rank(s, node);
    return l == UKL_AUX_NONE ? UKL_AUX_NONE : ukl_aux_term_of_leaf(s, l);
}

// Term `term`'s entry's offset, or 0 (no VOCAB, or no such term).
UKL_INLINE uint32_t ukl_aux_term_at(UKL_AUX_ARGS, uint32_t term)
{
    uint32_t sec = ukl_aux_section(s, UKL_AUX_VOCAB);
    if (sec == 0u || term >= ukl_aux_terms(s))
        return 0u;
    return sec + ukl_aux_u32(s, sec + 4u) + 16u * term;
}
// Term `term`'s hash (0 where none), its first occurrence's start and length in the source (`UKL_AUX_NONE`, 0).
UKL_INLINE uint64_t ukl_aux_term_hash(UKL_AUX_ARGS, uint32_t term)
{
    uint32_t e = ukl_aux_term_at(s, term);
    return e == 0u ? 0u : ukl_aux_u64(s, e);
}
UKL_INLINE uint32_t ukl_aux_term_start(UKL_AUX_ARGS, uint32_t term)
{
    uint32_t e = ukl_aux_term_at(s, term);
    return e == 0u ? UKL_AUX_NONE : ukl_aux_u32(s, e + 8u);
}
UKL_INLINE uint32_t ukl_aux_term_len(UKL_AUX_ARGS, uint32_t term)
{
    uint32_t e = ukl_aux_term_at(s, term);
    return e == 0u ? 0u : ukl_aux_u32(s, e + 12u);
}

// The term at position `k` in hash order, or `UKL_AUX_NONE`.
UKL_INLINE uint32_t ukl_aux_by_hash(UKL_AUX_ARGS, uint32_t k)
{
    uint32_t sec = ukl_aux_section(s, UKL_AUX_VOCAB);
    if (sec == 0u || k >= ukl_aux_terms(s))
        return UKL_AUX_NONE;
    return ukl_aux_u32(s, sec + ukl_aux_u32(s, sec + 8u) + 4u * k);
}

// The first position in hash order whose term's hash is at least `h` (the terms' count where none is).
UKL_INLINE uint32_t ukl_aux_hash_lower_bound(UKL_AUX_ARGS, uint64_t h)
{
    uint32_t sec = ukl_aux_section(s, UKL_AUX_VOCAB);
    if (sec == 0u)
        return 0u;
    uint32_t by = sec + ukl_aux_u32(s, sec + 8u), entries = sec + ukl_aux_u32(s, sec + 4u);
    uint32_t lo = 0u, hi = ukl_aux_terms(s);
    while (lo < hi) {
        uint32_t mid = (lo + hi) / 2u;
        if (ukl_aux_u64(s, entries + 16u * ukl_aux_u32(s, by + 4u * mid)) < h)
            lo = mid + 1u;
        else
            hi = mid;
    }
    return lo;
}

// The first term (in hash order) whose hash is `h`, or `UKL_AUX_NONE`: a lookup by a string's hash. Where two texts
// of a document share a hash, the next positions hold the others (`ukl_aux_by_hash`), and the caller compares text.
UKL_INLINE uint32_t ukl_aux_term_by_hash(UKL_AUX_ARGS, uint64_t h)
{
    uint32_t t = ukl_aux_by_hash(s, ukl_aux_hash_lower_bound(s, h));
    return t != UKL_AUX_NONE && ukl_aux_term_hash(s, t) == h ? t : UKL_AUX_NONE;
}

// A text's hash, as the auxiliary index keys a term (`aux_index::text_hash`): the `len` bytes at `start` of `src`, eight at a
// time (the last zero-filled), each folded in by a multiply, the length first and a finish after.
UKL_INLINE uint64_t ukl_aux_text_hash(UKL_GLOBAL const uint8_t* src, uint32_t start, uint32_t len)
{
    uint64_t h = 0x9E3779B97F4A7C15ul ^ ((uint64_t)len * 0xFF51AFD7ED558CCDul);
    for (uint32_t at = 0u; at < len; at += 8u) {
        uint32_t n = ukl_min(len - at, 8u);
        uint64_t w = ukl_ld_window(src, start + at, n);
        if (n < 8u)
            w &= (((uint64_t)1) << (8u * n)) - 1u;
        h = (h ^ w) * 0xBF58476D1CE4E5B9ul;
        h ^= h >> 29;
    }
    h ^= h >> 33;
    h *= 0xC4CEB9FE1A85EC53ul;
    return h ^ (h >> 33);
}

// ---- SIBLINGS ----

UKL_INLINE uint32_t ukl_aux_pair_parent(UKL_AUX_ARGS, uint32_t sec, uint32_t i) { return ukl_aux_u32(s, sec + 16u + 8u * i); }
UKL_INLINE uint32_t ukl_aux_pair_node(UKL_AUX_ARGS, uint32_t sec, uint32_t i) { return ukl_aux_u32(s, sec + 20u + 8u * i); }

// The first SIBLINGS entry whose parent is at least `parent`.
UKL_INLINE uint32_t ukl_aux_family(UKL_AUX_ARGS, uint32_t sec, uint32_t parent)
{
    uint32_t lo = 0u, hi = ukl_aux_u32(s, sec);
    while (lo < hi) {
        uint32_t mid = (lo + hi) / 2u;
        if (ukl_aux_pair_parent(s, sec, mid) < parent)
            lo = mid + 1u;
        else
            hi = mid;
    }
    return lo;
}

// The node of `parent`'s child `K * k` (k >= 1), or `UKL_AUX_NONE`.
UKL_INLINE uint32_t ukl_aux_eighth(UKL_AUX_ARGS, uint32_t parent, uint32_t k)
{
    uint32_t sec = ukl_aux_section(s, UKL_AUX_SIBLINGS);
    if (sec == 0u || k == 0u)
        return UKL_AUX_NONE;
    uint32_t at = ukl_aux_family(s, sec, parent) + k - 1u;
    return at < ukl_aux_u32(s, sec) && ukl_aux_pair_parent(s, sec, at) == parent ? ukl_aux_pair_node(s, sec, at) : UKL_AUX_NONE;
}

// The last of `parent`'s every-K-th children at or before `node`: its k in the high 32 bits and its node in the low,
// or `UKL_AUX_NONE` in the low bits (and k 0) where none is.
UKL_INLINE uint64_t ukl_aux_eighth_at_or_before(UKL_AUX_ARGS, uint32_t parent, uint32_t node)
{
    uint32_t sec = ukl_aux_section(s, UKL_AUX_SIBLINGS);
    if (sec == 0u)
        return UKL_AUX_NONE;
    uint32_t first = ukl_aux_family(s, sec, parent);
    uint32_t lo = first, hi = ukl_aux_family(s, sec, parent == UKL_AUX_NONE ? parent : parent + 1u);
    while (lo < hi) {
        uint32_t mid = (lo + hi) / 2u;
        if (ukl_aux_pair_node(s, sec, mid) <= node)
            lo = mid + 1u;
        else
            hi = mid;
    }
    if (lo == first)
        return UKL_AUX_NONE;
    return ((uint64_t)(lo - first) << 32) | ukl_aux_pair_node(s, sec, lo - 1u);
}

// ---- COUNTS and RARE ----

// The nodes of code `code` (0 where it has none), or `UKL_AUX_NONE` (no COUNTS).
UKL_INLINE uint32_t ukl_aux_count(UKL_AUX_ARGS, uint32_t code)
{
    uint32_t sec = ukl_aux_section(s, UKL_AUX_COUNTS);
    if (sec == 0u)
        return UKL_AUX_NONE;
    uint32_t lo = 0u, hi = ukl_aux_u32(s, sec);
    while (lo < hi) {
        uint32_t mid = (lo + hi) / 2u;
        uint32_t c = ukl_aux_u32(s, sec + 16u + 8u * mid);
        if (c == code)
            return ukl_aux_u32(s, sec + 20u + 8u * mid);
        if (c < code)
            lo = mid + 1u;
        else
            hi = mid;
    }
    return 0u;
}

// Code `code`'s nodes' positions in RARE: the first in the low 32 bits, one past the last in the high; `UKL_AUX_NONE`
// in both where the code is not rare (or no RARE). A code absent from the document is rare, with none.
UKL_INLINE uint64_t ukl_aux_rare_range(UKL_AUX_ARGS, uint32_t code)
{
    uint64_t none = ((uint64_t)UKL_AUX_NONE << 32) | UKL_AUX_NONE;
    uint32_t sec = ukl_aux_section(s, UKL_AUX_RARE);
    if (sec == 0u)
        return none;
    uint32_t lo = 0u, hi = ukl_aux_u32(s, sec);
    while (lo < hi) {
        uint32_t mid = (lo + hi) / 2u;
        uint32_t c = ukl_aux_u32(s, sec + 16u + 8u * mid);
        if (c == code)
            return ((uint64_t)ukl_aux_u32(s, sec + 28u + 8u * mid) << 32) | ukl_aux_u32(s, sec + 20u + 8u * mid);
        if (c < code)
            lo = mid + 1u;
        else
            hi = mid;
    }
    uint32_t n = ukl_aux_count(s, code);
    return n != UKL_AUX_NONE && n > 0u ? none : 0u;
}

// RARE's node at position `k`.
UKL_INLINE uint32_t ukl_aux_rare_node(UKL_AUX_ARGS, uint32_t k)
{
    uint32_t sec = ukl_aux_section(s, UKL_AUX_RARE);
    return sec == 0u ? UKL_AUX_NONE : ukl_aux_u32(s, sec + ukl_aux_u32(s, sec + 8u) + 4u * k);
}
