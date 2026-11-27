// ukl/flat.h: the flat layer's working forms (`rerail_delta_core::flat`, *Flat Views and Lifecycles* step 7) claimed,
// grown and read by a kernel over the same bytes the Rust forms are: an operation's phase runs on a CPU or a GPU
// over one block's bytes, and what either writes the other reads.
//
// A region is a byte buffer `r` whose base is 64-byte aligned. Every structure lies at a byte offset in it, and its
// references (`Rel<u8, B32>`) are self-relative: a signed 32-bit count of bytes from the reference's own field, 0 for
// none. So a region moves by `memcpy`, and its offsets mean the same on the host and the device.
//
// Each header's layout is its Rust `structure!`'s (the offsets below; rerail-delta-device's `flat_tests` runs one
// script of claims in Rust and here over copies of one region and compares the bytes). A claim that does not fit is
// `UKL_NONE`, its failure recorded where the Rust form records it (an arena's header; a vector's length counting on),
// so the owner sizes the room again and the phase runs again (*0686: Operations* §6).
//
// Only 32-bit atomics: every backend has them, and Metal has no 64-bit compare-and-swap. So the map here is the 32-bit
// one (`Map<u32, u32>`); the 64-bit-keyed map and `Stamped` (a slot's round and key in one 64-bit word) are the host's
// until they have a two-word claim.

#pragma once
#include <ukl/bits.h>

// An arena's header (`ArenaHead`, 32 bytes): its magic, bucket, top (its claims' bytes, counting on past its cap),
// cap, first failure's part plus one, that failure's need, round. Its claims start past it.
#define UKL_FLAT_HEAD 32u
#define UKL_FLAT_ARENA_TOP 8u
#define UKL_FLAT_ARENA_CAP 12u
#define UKL_FLAT_ARENA_FAIL 16u
#define UKL_FLAT_ARENA_FAIL_NEED 20u
// Every claim is a whole number of grains; no claim is past this many bytes.
#define UKL_FLAT_GRAIN 8u
#define UKL_FLAT_MAX_CAP 0x80000000u
// A map's claim that put its key in the slot it gives (slots are under 2^30 in a region under 4 GB).
#define UKL_FLAT_FRESH 0x80000000u

// The 32-bit word at byte `at` of the region (4-byte aligned).
UKL_INLINE UKL_GLOBAL uint32_t* ukl_flat_word(UKL_GLOBAL uint8_t* r, uint32_t at) { return (UKL_GLOBAL uint32_t*)(r + at); }

// The target of the byte reference at `field` (`Rel<_, B32>`): its offset in the region, or `UKL_NONE` for none.
UKL_INLINE uint32_t ukl_flat_rel(UKL_GLOBAL uint8_t* r, uint32_t field)
{
    int32_t u = (int32_t)ukl_atomic_load(ukl_flat_word(r, field));
    return u == 0 ? UKL_NONE : (uint32_t)((int32_t)field + u);
}

// ---- the arena: one atomic add on its top ------------------------------------------------------------------------

// The arena at `a` failed a claim of part `part` that needed `need` bytes: the first failure is kept (`Arena::fail`).
UKL_INLINE void ukl_flat_arena_fail(UKL_GLOBAL uint8_t* r, uint32_t a, uint32_t part, uint32_t need)
{
    if (ukl_atomic_cas(ukl_flat_word(r, a + UKL_FLAT_ARENA_FAIL), 0u, part + 1u) == 0u)
        ukl_atomic_store(ukl_flat_word(r, a + UKL_FLAT_ARENA_FAIL_NEED), need);
}

// `bytes` at `align` (a power of two, at most 64) claimed in the arena whose header lies at `a` (64-byte aligned):
// the claim's offset in the region, or `UKL_NONE` (its failure recorded, its top counted on). `Arena::claim_at`.
UKL_INLINE uint32_t ukl_flat_claim(UKL_GLOBAL uint8_t* r, uint32_t a, uint32_t part, uint32_t bytes, uint32_t align)
{
    uint32_t pad = align > UKL_FLAT_GRAIN ? align - UKL_FLAT_GRAIN : 0u;
    uint64_t room = (((uint64_t)bytes + (UKL_FLAT_GRAIN - 1u)) & ~(uint64_t)(UKL_FLAT_GRAIN - 1u)) + pad;
    if (room > UKL_FLAT_MAX_CAP) {
        ukl_flat_arena_fail(r, a, part, bytes);
        return UKL_NONE;
    }
    uint32_t old = ukl_atomic_add(ukl_flat_word(r, a + UKL_FLAT_ARENA_TOP), (uint32_t)room);
    if ((uint64_t)old + room > (uint64_t)ukl_atomic_load(ukl_flat_word(r, a + UKL_FLAT_ARENA_CAP))) {
        ukl_flat_arena_fail(r, a, part, (uint32_t)room);
        return UKL_NONE;
    }
    uint32_t start = UKL_FLAT_HEAD + old;
    return a + ((start + align - 1u) & ~(align - 1u));
}

// ---- a growing vector (`GrowHead`, 20 bytes): its room, length (counting on), length that fit, cap, part ---------

// `n` items of `size` bytes claimed in the vector at `g` by one atomic add: the first's offset, or `UKL_NONE` past its
// room, its length counting on (`Grow::wanted`) for its owner to grow it to. `Grow::claim`.
UKL_INLINE uint32_t ukl_flat_grow_claim(UKL_GLOBAL uint8_t* r, uint32_t g, uint32_t n, uint32_t size)
{
    uint32_t old = ukl_atomic_add(ukl_flat_word(r, g + 4u), n);
    uint64_t end = (uint64_t)old + n;
    if (end > (uint64_t)ukl_atomic_load(ukl_flat_word(r, g + 12u)))
        return UKL_NONE;
    ukl_atomic_max(ukl_flat_word(r, g + 8u), (uint32_t)end);
    return ukl_flat_rel(r, g) + old * size;
}

// Items of the vector at `g`: the claims that fit (`Grow::len`).
UKL_INLINE uint32_t ukl_flat_grow_len(UKL_GLOBAL uint8_t* r, uint32_t g) { return ukl_atomic_load(ukl_flat_word(r, g + 8u)); }

// ---- a pool (`PoolHead`, 24 bytes): its items, its free stack, cap, items handed out, items free, part -----------

// An item of the pool at `p`, 1-based: the free stack's top, popped by compare-and-swap, else an untouched item by
// one atomic add; `UKL_NONE` past its cap. `Pool::take`. (Each failed swap saw the stack shorter: at most its depth.)
UKL_INLINE uint32_t ukl_flat_pool_take(UKL_GLOBAL uint8_t* r, uint32_t p)
{
    UKL_GLOBAL uint32_t* nfree = ukl_flat_word(r, p + 16u);
    uint32_t n = ukl_atomic_load(nfree);
    while (n > 0u) {
        uint32_t seen = ukl_atomic_cas(nfree, n, n - 1u);
        if (seen == n)
            return ukl_atomic_load(ukl_flat_word(r, ukl_flat_rel(r, p + 4u) + 4u * (n - 1u)));
        n = seen;
    }
    uint32_t i = ukl_atomic_add(ukl_flat_word(r, p + 12u), 1u);
    return i >= ukl_atomic_load(ukl_flat_word(r, p + 8u)) ? UKL_NONE : i + 1u;
}

// Item `i`'s offset (1-based) in the pool at `p`, of items of `size` bytes.
UKL_INLINE uint32_t ukl_flat_pool_item(UKL_GLOBAL uint8_t* r, uint32_t p, uint32_t i, uint32_t size) { return ukl_flat_rel(r, p) + (i - 1u) * size; }

// Item `i` back onto the free stack of the pool at `p`, by its one owner (`Pool::give`, `&mut`): no other lane takes
// or gives meanwhile.
UKL_INLINE void ukl_flat_pool_give(UKL_GLOBAL uint8_t* r, uint32_t p, uint32_t i)
{
    UKL_GLOBAL uint32_t* nfree = ukl_flat_word(r, p + 16u);
    uint32_t n = ukl_atomic_load(nfree);
    ukl_atomic_store(ukl_flat_word(r, ukl_flat_rel(r, p + 4u) + 4u * n), i);
    ukl_atomic_store(nfree, n + 1u);
}

// ---- a chain (`ChainHead`, 28 bytes): a pool of its items, then each item's next link ----------------------------

// The word holding link `i`'s next, in the chain at `c`.
UKL_INLINE UKL_GLOBAL uint32_t* ukl_flat_chain_next(UKL_GLOBAL uint8_t* r, uint32_t c, uint32_t i)
{
    return ukl_flat_word(r, ukl_flat_rel(r, c + 24u) + 4u * (i - 1u));
}

// A link taken in the chain at `c` (its value written at `ukl_flat_pool_item(r, c, i, size)`), `UKL_NONE` past its cap.
UKL_INLINE uint32_t ukl_flat_chain_take(UKL_GLOBAL uint8_t* r, uint32_t c) { return ukl_flat_pool_take(r, c); }

// Link `i` (its value written) put before `tail`: a list one lane owns (`Chain::cons`).
UKL_INLINE void ukl_flat_chain_cons(UKL_GLOBAL uint8_t* r, uint32_t c, uint32_t i, uint32_t tail) { ukl_atomic_store(ukl_flat_chain_next(r, c, i), tail); }

// Link `i` (its value written) pushed onto the list whose head is the word at `head`, which many lanes push onto at
// once (`Chain::push`).
UKL_INLINE void ukl_flat_chain_push(UKL_GLOBAL uint8_t* r, uint32_t c, uint32_t head, uint32_t i)
{
    UKL_GLOBAL uint32_t* h = ukl_flat_word(r, head);
    uint32_t t = ukl_atomic_load(h);
    for (;;) {
        ukl_atomic_store(ukl_flat_chain_next(r, c, i), t);
        uint32_t seen = ukl_atomic_cas(h, t, i);
        if (seen == t)
            return;
        t = seen;
    }
}

// ---- a 32-bit map (`MapHead`, 32 bytes): keys, values, mask, probe bound, length, part, empty key ----------------

// A key's hash, as `Word::hash` makes it: the CLIF's `rel_key_mix` from the seed (`rerail_delta_core::hash::mix`),
// its low 32 bits.
UKL_INLINE uint32_t ukl_flat_hash_u32(uint32_t k)
{
    uint64_t h = (0x243F6A8885A308D3ull ^ (uint64_t)k) * 0x9E3779B97F4A7C15ull;
    return (uint32_t)(h ^ (h >> 29));
}

// `k`'s slot in the map at `m`, claiming a free one by compare-and-swap if it has none (`| UKL_FLAT_FRESH` when this
// claim put it there); `UKL_NONE` when no free slot lies within the probe bound (`Full`: a rehash phase moves it to
// a larger map). `Map::claim`.
UKL_INLINE uint32_t ukl_flat_map_claim(UKL_GLOBAL uint8_t* r, uint32_t m, uint32_t k)
{
    uint32_t keys = ukl_flat_rel(r, m);
    uint32_t mask = ukl_atomic_load(ukl_flat_word(r, m + 8u)), bound = ukl_atomic_load(ukl_flat_word(r, m + 12u));
    uint32_t empty = ukl_atomic_load(ukl_flat_word(r, m + 24u));
    uint32_t h = ukl_flat_hash_u32(k);
    for (uint32_t i = 0u; i < bound; i++) {
        uint32_t s = (h + i) & mask;
        UKL_GLOBAL uint32_t* at = ukl_flat_word(r, keys + 4u * s);
        uint32_t cur = ukl_atomic_load(at);
        if (cur == k)
            return s;
        if (cur == empty) {
            uint32_t seen = ukl_atomic_cas(at, empty, k);
            if (seen == empty) {
                ukl_atomic_add(ukl_flat_word(r, m + 16u), 1u);
                return s | UKL_FLAT_FRESH;
            }
            if (seen == k)
                return s;
        }
    }
    return UKL_NONE;
}

// `k`'s slot in the map at `m`, or `UKL_NONE`. `Map::find`.
UKL_INLINE uint32_t ukl_flat_map_find(UKL_GLOBAL uint8_t* r, uint32_t m, uint32_t k)
{
    uint32_t keys = ukl_flat_rel(r, m);
    uint32_t mask = ukl_atomic_load(ukl_flat_word(r, m + 8u)), bound = ukl_atomic_load(ukl_flat_word(r, m + 12u));
    uint32_t empty = ukl_atomic_load(ukl_flat_word(r, m + 24u));
    uint32_t h = ukl_flat_hash_u32(k);
    for (uint32_t i = 0u; i < bound; i++) {
        uint32_t s = (h + i) & mask;
        uint32_t cur = ukl_atomic_load(ukl_flat_word(r, keys + 4u * s));
        if (cur == k && k != empty)
            return s;
        if (cur == empty)
            return UKL_NONE;
    }
    return UKL_NONE;
}

// The word holding slot `s`'s value in the map at `m`.
UKL_INLINE UKL_GLOBAL uint32_t* ukl_flat_map_val(UKL_GLOBAL uint8_t* r, uint32_t m, uint32_t s)
{
    return ukl_flat_word(r, ukl_flat_rel(r, m + 4u) + 4u * (s & ~UKL_FLAT_FRESH));
}

// `k` claimed in the map at `m` with its least value kept, `v` or one before it: the same map in any order of lanes
// (`Map::claim_min`). The claim's slot, or `UKL_NONE`.
UKL_INLINE uint32_t ukl_flat_map_claim_min(UKL_GLOBAL uint8_t* r, uint32_t m, uint32_t k, uint32_t v)
{
    uint32_t c = ukl_flat_map_claim(r, m, k);
    if (c != UKL_NONE)
        ukl_atomic_min(ukl_flat_map_val(r, m, c), v);
    return c;
}

// `k`'s value set to `v` in the map at `m` (the last writer's stays): the claim's slot, or `UKL_NONE`. `Map::set`.
UKL_INLINE uint32_t ukl_flat_map_set(UKL_GLOBAL uint8_t* r, uint32_t m, uint32_t k, uint32_t v)
{
    uint32_t c = ukl_flat_map_claim(r, m, k);
    if (c != UKL_NONE)
        ukl_atomic_store(ukl_flat_map_val(r, m, c), v);
    return c;
}
