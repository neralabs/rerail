// ukl/hash.h: a hash table of 32-bit keys built and probed on the device (decision 0400 §5).
//
// Open addressing with linear probing, a power-of-two capacity. A slot's key is `UKL_HASH_EMPTY` until a thread
// claims it by compare-and-swap; a key is never moved or removed, so a probe that meets an empty slot knows the key is
// absent. Every probe loop is bounded by the capacity: a full table is reported (`UKL_NONE`), never looped on.
// The key `UKL_HASH_EMPTY` itself cannot be stored.
//
// The build kernel keeps, per key, its first row (an atomic minimum: the same table whatever order the threads run
// in) and its rows' count; the probe kernel gives each query its key's first row, or `UKL_NONE`. 64-bit keys need a
// two-word claim (Metal has no 64-bit compare-and-swap): not yet here.

#pragma once
#include <ukl/bits.h>

#define UKL_HASH_EMPTY 0xFFFFFFFFu

// murmur3's finalizer: every bit of the key reaches every bit of the hash.
UKL_INLINE uint32_t ukl_hash_u32(uint32_t x)
{
    x ^= x >> 16;
    x *= 0x85ebca6bu;
    x ^= x >> 13;
    x *= 0xc2b2ae35u;
    x ^= x >> 16;
    return x;
}

// The slot of `key`, claiming an empty one when it is absent; `UKL_NONE` when the table is full. `mask` is the
// capacity less one.
UKL_INLINE uint32_t ukl_hash_insert(UKL_GLOBAL uint32_t* keys, uint32_t mask, uint32_t key)
{
    uint32_t s = ukl_hash_u32(key) & mask;
    for (uint32_t k = 0u; k <= mask; k++) {
        uint32_t seen = ukl_atomic_load(&keys[s]);
        if (seen == UKL_HASH_EMPTY)
            seen = ukl_atomic_cas(&keys[s], UKL_HASH_EMPTY, key);
        // Ours now (the swap saw it empty), or already the key's.
        if (seen == UKL_HASH_EMPTY || seen == key)
            return s;
        s = (s + 1u) & mask;
    }
    return UKL_NONE;
}

// The slot of `key`, or `UKL_NONE` when it is absent.
UKL_INLINE uint32_t ukl_hash_find(UKL_GLOBAL const uint32_t* keys, uint32_t mask, uint32_t key)
{
    uint32_t s = ukl_hash_u32(key) & mask;
    for (uint32_t k = 0u; k <= mask; k++) {
        uint32_t seen = keys[s];
        if (seen == key)
            return s;
        if (seen == UKL_HASH_EMPTY)
            return UKL_NONE;
        s = (s + 1u) & mask;
    }
    return UKL_NONE;
}

struct UklHashArgs {
    uint32_t n;
    uint32_t mask;
};
UKL_REFLECT_STRUCT(UklHashArgs, n, mask)

// Row `i`'s key into the table: the key's first row (`first`, starting at `UKL_NONE`) and its count (from 0). A row
// that finds the table full sets `full`.
DEFINE_KERNEL2(ukl, ukl_hash_build_u32,
               ARGS,     UklHashArgs, args,   ,
               ROBUFFER, uint32_t,    rows,   "[n]",
               RWBUFFER, uint32_t,    keys,   "[mask + 1]",
               RWBUFFER, uint32_t,    first,  "[mask + 1]",
               RWBUFFER, uint32_t,    counts, "[mask + 1]",
               RWBUFFER, uint32_t,    full,   "[1]",
               GID0,     uint32_t,    i,      )
{
    if (i >= args.n)
        return;
    uint32_t s = ukl_hash_insert(keys, args.mask, rows[i]);
    if (s == UKL_NONE) {
        ukl_atomic_store(full, 1u);
        return;
    }
    ukl_atomic_min(&first[s], i);
    ukl_atomic_add(&counts[s], 1u);
}

// Query `i`'s key's slot's value (`values[slot]`), or `UKL_NONE` when the key is absent.
DEFINE_KERNEL2(ukl, ukl_hash_probe_u32,
               ARGS,     UklHashArgs, args,    ,
               ROBUFFER, uint32_t,    queries, "[n]",
               ROBUFFER, uint32_t,    keys,    "[mask + 1]",
               ROBUFFER, uint32_t,    values,  "[mask + 1]",
               RWBUFFER, uint32_t,    out,     "[n]",
               GID0,     uint32_t,    i,       )
{
    if (i >= args.n)
        return;
    uint32_t s = ukl_hash_find(keys, args.mask, queries[i]);
    out[i] = s == UKL_NONE ? UKL_NONE : values[s];
}
