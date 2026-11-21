// ukl/pool.h: a store's blocks used again, a free list a size class (decision 0486 §10).
//
// A store (an array of units: tokens, rows, ...) handed out from its front by an atomic add never takes back what
// its users are done with. A pool keeps those blocks to hand out again: a free list a size class, each list linked
// through a word of its blocks' own (`next`, indexed by a block's first unit: the store's own column, or any word
// array as long as the store), so it needs no room of its own beyond its heads.
//
// The pool is used in two phases, never at once, and a barrier between them (a grid barrier, or a dispatch's end):
// - giving (`ukl_pool_give`, `ukl_pool_give_range`): blocks pushed, by exchange on their class's head;
// - taking (`ukl_pool_take`): blocks popped, by compare-and-swap on the head.
// No block is pushed while another is popped, so no head comes back to a block a taker read (no ABA), with 32-bit
// atomics alone, and a pop's read of `next` may find it overwritten only where the block was popped already, so its
// swap fails. A take that finds its class empty (or its swaps spent, `UKL_POOL_TURNS`) returns `UKL_POOL_NONE`, and
// the caller takes from the store's front.
//
// A split take (`ukl_pool_take_split`) takes from the smallest class at least its own that holds a block, and
// pushes the rest of a larger block back while others take (`ukl_pool_push`: the link written before the swap that
// publishes it). That is still no ABA: each piece begins inside the block the taker popped, past its first unit,
// and no other block holding that unit is in the pool, so no block is pushed to a class twice between two givings,
// and no head comes back to a block a taker read.
//
// The classes: powers of two (class `c` is 2^c units). A block handed out from the front is its class's size (at
// most twice what was asked), so it fits its class's next taker exactly. A range given back that is no class's size
// is given as pieces, each the largest power of two it still holds (its bits, high to low).
//
// Later, classes finer where blocks are small and coarser where they are large (multiples of 8 bytes, then of a
// cache line, then of a page) may cut what rounding up wastes; the class functions below are all that changes.

#pragma once
#include <ukl/bits.h>

#define UKL_POOL_NONE 0xFFFFFFFFu
// The classes: 2^0 to 2^31.
#define UKL_POOL_CLASSES 32u
// A take's swaps at most.
#ifndef UKL_POOL_TURNS
#define UKL_POOL_TURNS 64u
#endif

typedef struct UklPool {
    // A head a class (`UKL_POOL_CLASSES`, `UKL_POOL_NONE` when empty), and the links.
    UKL_GLOBAL uint32_t* heads;
    UKL_GLOBAL uint32_t* next;
} UklPool;

// Class `c`'s size.
UKL_INLINE uint32_t ukl_pool_size(uint32_t c) { return 1u << c; }

// The largest class at most `n` units (`n` at least 1).
UKL_INLINE uint32_t ukl_pool_floor(uint32_t n) { return 31u - ukl_clz32(n); }

// The smallest class of at least `n` units (`n` at least 1, at most 2^31).
UKL_INLINE uint32_t ukl_pool_ceil(uint32_t n)
{
    uint32_t c = ukl_pool_floor(n);
    return ukl_pool_size(c) < n ? c + 1u : c;
}

// Every class empty (before the first give, by one thread, or each head by its own).
UKL_INLINE void ukl_pool_clear(UklPool p, uint32_t c) { ukl_atomic_store(&p.heads[c], UKL_POOL_NONE); }

// Giving: the block at `start`, of class `c`'s size.
UKL_INLINE void ukl_pool_give(UklPool p, uint32_t start, uint32_t c)
{
    uint32_t old = ukl_atomic_exchange(&p.heads[c], start);
    ukl_atomic_store(&p.next[start], old);
}

// Giving: the `n` units at `start`, as pieces of the largest classes they hold, largest first. The pieces given.
UKL_INLINE uint32_t ukl_pool_give_range(UklPool p, uint32_t start, uint32_t n)
{
    uint32_t pieces = 0u;
    // A piece a set bit of `n`: 32 turns cover 32-bit counts.
    for (uint32_t turn = 0u; turn < 32u && n != 0u; turn++) {
        uint32_t c = ukl_pool_floor(n), size = ukl_pool_size(c);
        ukl_pool_give(p, start, c);
        start += size;
        n -= size;
        pieces++;
    }
    return pieces;
}

// Pushing while others take: the block at `start`, of class `c`'s size, its link written and made visible before
// the swap that makes it the head. Past `UKL_POOL_TURNS` swaps the block is not pushed (it is lost to the pool,
// never handed out twice).
UKL_INLINE void ukl_pool_push(UklPool p, uint32_t start, uint32_t c)
{
    uint32_t head = ukl_atomic_load(&p.heads[c]);
    for (uint32_t turn = 0u; turn < UKL_POOL_TURNS; turn++) {
        ukl_atomic_store(&p.next[start], head);
        ukl_fence_device();
        uint32_t seen = ukl_atomic_cas(&p.heads[c], head, start);
        if (seen == head)
            return;
        head = seen;
    }
}

// Taking: a block of class `c` (its first unit), or `UKL_POOL_NONE`.
UKL_INLINE uint32_t ukl_pool_take(UklPool p, uint32_t c)
{
    uint32_t head = ukl_atomic_load(&p.heads[c]);
    for (uint32_t turn = 0u; turn < UKL_POOL_TURNS && head != UKL_POOL_NONE; turn++) {
        uint32_t next = ukl_atomic_load(&p.next[head]);
        uint32_t seen = ukl_atomic_cas(&p.heads[c], head, next);
        if (seen == head)
            return head;
        head = seen;
    }
    return UKL_POOL_NONE;
}

// Taking a block of class `c` from the smallest class at least `c` that holds one: of a larger block, its first
// 2^c units, the rest pushed back as pieces of 2^c, 2^(c+1), ... (`ukl_pool_push`). `UKL_POOL_NONE` when none
// was found (each class tried once).
UKL_INLINE uint32_t ukl_pool_take_split(UklPool p, uint32_t c)
{
    for (uint32_t k = c; k < UKL_POOL_CLASSES; k++) {
        if (ukl_atomic_load(&p.heads[k]) == UKL_POOL_NONE)
            continue;
        uint32_t s = ukl_pool_take(p, k);
        if (s == UKL_POOL_NONE)
            continue;
        for (uint32_t j = c; j < k; j++)
            ukl_pool_push(p, s + ukl_pool_size(j), j);
        return s;
    }
    return UKL_POOL_NONE;
}
