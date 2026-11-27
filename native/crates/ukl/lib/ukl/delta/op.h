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
// params' bytes, its tile of items, and one result word (a phase's item count, or whether it runs again).
#define DELTA_KERNEL(name)                                                                                          \
    DEFINE_KERNEL2(delta_exec, name, ROBUFFER, uint8_t, t, "[table]", ROBUFFER, uint32_t, slots, "[table]",         \
                   ROBUFFER, uint8_t, p, "[table]", ARGS, TileP, tile, , RWBUFFER, uint64_t, res, "[1]", GID0,       \
                   uint32_t, i, )

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
