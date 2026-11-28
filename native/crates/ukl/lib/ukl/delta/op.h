// ukl/delta/op.h: what every kernel of a delta operation's UKL version shares (decision 0686): its head, its slots'
// blocks, and a block's words, read and written where they lie. The layouts are the twins' (`<ukl/delta/layout.h>`,
// generated from them). Included first by every `delta_exec/ukl/*.uglykl`.
#pragma once
#include <ukl/clif.h>
#include <ukl/delta/layout.h>

// A block's bytes, where they lie: a slot's, or one a slot refers to, found by its id in the launch's table.
typedef CLIF_BLOCK_SPACE uint8_t* Blk;
typedef CLIF_BLOCK_SPACE const uint8_t* CBlk;

// Every kernel takes the same arguments (`rerail_delta_device`): the launch's device table, the slots' block ids, the
// params' bytes, its tile of items, one result word (a phase's item count, or whether it runs again), and its meter:
// its slots' capacities and the room its items need (`reach`).
#define DELTA_KERNEL(name)                                                                                          \
    DEFINE_KERNEL2(delta_exec, name, ROBUFFER, uint8_t, t, "[table]", ROBUFFER, uint32_t, slots, "[table]",         \
                   ROBUFFER, uint8_t, p, "[table]", ARGS, TileP, tile, , RWBUFFER, uint64_t, res, "[1]", RWBUFFER,  \
                   uint32_t, meter, "[meter]", GID0, uint32_t, i, )

// A phase of one item: its count kernel, then the item's kernel's head.
#define DELTA_ONE(name)                                                                                             \
    DELTA_KERNEL(name##_items) { if (i == 0u) res[0] = 1u; }                                                        \
    DELTA_KERNEL(name)

// The item a thread runs, or return past its tile.
#define DELTA_ITEM(k) uint32_t k = tile.lo + i; if (k >= tile.hi) return

// Block `id`'s bytes, and slot `k`'s.
UKL_INLINE Blk blk(CLIF_BLOCK_SPACE const uint8_t* t, uint32_t id) { return (Blk)desc_row(t, id)->addr; }
#define SLOT(k) blk(t, slots[k])

// The params, as their struct.
#define PARAMS(T) (*(UKL_GLOBAL const T*)p)

// A word at byte `at` of a block, aligned.
#define U32(b, at) (*(CLIF_BLOCK_SPACE uint32_t*)((b) + (at)))
#define I32(b, at) (*(CLIF_BLOCK_SPACE int32_t*)((b) + (at)))
#define U64(b, at) (*(CLIF_BLOCK_SPACE uint64_t*)((b) + (at)))
#define I64(b, at) (*(CLIF_BLOCK_SPACE int64_t*)((b) + (at)))

// Whether slot `k`'s block holds `end` bytes from its start (its capacity, `meter[k]`); else what it needs recorded
// (`meter[MAX_SLOTS + k]`), and the item writes nothing past the capacity: the executor grows the block within its
// reservation and runs the phase, or the launch, again (decision 0686 D6, the twins' `Ctx::reach`).
UKL_INLINE bool reach(UKL_GLOBAL uint32_t* meter, uint32_t k, uint64_t end) {
    if (end <= meter[k]) return true;
    ukl_atomic_max(&meter[MAX_SLOTS + k], end > 0xFFFFFFFFull ? 0xFFFFFFFFu : (uint32_t)end);
    return false;
}
#define REACH(k, end) reach(meter, k, end)

// The launch refused, by reason `why` (not 0): it is one its kernels do not do, and the host fails it, naming it. The
// word after the fronts (`meter[3 * MAX_SLOTS]`).
#define FAULT(why) (meter[3u * MAX_SLOTS] = (why))

// `bytes` of slot `k`'s block claimed at `align` (a power of two) from its front (`meter[2 * MAX_SLOTS + k]`): their
// offset in the block. Past its capacity, NONE: what it needs is recorded (`reach`), and nothing is claimed. The host
// takes the front back after the launch (`Block::claimed_to`): the claims are the block's, as `try_claim`'s are.
UKL_INLINE uint32_t claim(UKL_GLOBAL uint32_t* meter, uint32_t k, uint32_t bytes, uint32_t align) {
    UKL_GLOBAL uint32_t* front = &meter[2u * MAX_SLOTS + k];
    for (uint32_t at = *front;;) {
        uint32_t start = (at + align - 1u) & ~(align - 1u);
        if (!reach(meter, k, (uint64_t)start + bytes)) return NONE;
        uint32_t was = ukl_atomic_cas(front, at, start + bytes);
        if (was == at) return start;
        at = was;
    }
}
#define CLAIM(k, bytes, align) claim(meter, k, bytes, align)
