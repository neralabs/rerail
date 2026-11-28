// ukl/delta/op.h: what every kernel of a delta operation's UKL version shares (decision 0686): its head, its slots'
// blocks, and a block's words, read and written where they lie. The layouts are the twins' (`<ukl/delta/layout.h>`,
// generated from them). Included first by every `delta_exec/ukl/*.uglykl`.
#pragma once
#include <ukl/clif.h>
#include <ukl/group.h>
#include <ukl/delta/layout.h>

// A block's bytes, where they lie: a slot's, or one a slot refers to, found by its id in the launch's table.
typedef CLIF_BLOCK_SPACE uint8_t* Blk;
typedef CLIF_BLOCK_SPACE const uint8_t* CBlk;

// A launch's phases run as one command on the device (`rerail_delta_device`, `OnDevice::program`): each phase's count
// kernel writes its items and sizes the phase's dispatch from them, and the phase runs over that grid, with no wait
// for the host between. A phase that runs again (`Across::Rounds`) is queued up to its bound on turns, each followed by
// its `again` kernel, which sizes the next turn's dispatch, or none after the last.
//
// Every kernel takes the same arguments: the launch's device table, the slots' block ids, the params' bytes, its
// phase (`ph.k`), the launch's control words (`ctl`: each phase's count, whether it runs, its grid, its turns run) and
// its meter: its slots' capacities, the room its items need (`reach`), its fronts (`claim`), its refusal and whether
// it overflowed.
#define DELTA_PHASES 32u
#define DELTA_COUNT_AT 0u
#define DELTA_GO_AT DELTA_PHASES
#define DELTA_GRID_AT (2u * DELTA_PHASES)
#define DELTA_TURNS_AT (5u * DELTA_PHASES)
#define DELTA_CTL_WORDS (6u * DELTA_PHASES)
// The meter's words past the fronts: the refusal (`FAULT`), and whether a claim or a reach overflowed.
#define DELTA_FAULT_AT (3u * MAX_SLOTS)
#define DELTA_OVER_AT (3u * MAX_SLOTS + 1u)
// A threadgroup's threads, where a phase's item is a threadgroup's (`Unit::Threadgroup`), and its words of group
// memory beside `ukl/group.h`'s scratch.
#define DELTA_TG 64u
#define DELTA_SHARED 1024u

#define DELTA_ARGS(name, ...)                                                                                       \
    DEFINE_KERNEL2(delta_exec, name, ROBUFFER, uint8_t, t, "[table]", ROBUFFER, uint32_t, slots, "[table]",         \
                   ROBUFFER, uint8_t, p, "[table]", ARGS, PhaseP, ph, , RWBUFFER, uint32_t, ctl, "[ctl]",           \
                   RWBUFFER, uint32_t, meter, "[meter]", __VA_ARGS__)

// Whether the launch is over: refused, or a slot found too small (it runs again, grown: no phase after does work).
#define DELTA_STOPPED (meter[DELTA_FAULT_AT] != 0u || meter[DELTA_OVER_AT] != 0u)
// This phase's items, and whether it runs (a round phase stops after its last turn).
#define DELTA_N ctl[DELTA_COUNT_AT + ph.k]
#define DELTA_GO (ctl[DELTA_GO_AT + ph.k] != 0u)

// The arguments every kernel takes, as a function of the kernel's takes them.
#define DELTA_KPARAMS                                                                                               \
    UKL_GLOBAL const uint8_t *t, UKL_GLOBAL const uint32_t *slots, UKL_GLOBAL const uint8_t *p, PhaseP ph,          \
        UKL_GLOBAL uint32_t *ctl, UKL_GLOBAL uint32_t *meter
#define DELTA_KARGS t, slots, p, ph, ctl, meter

// Phase `ph`'s items, `n`, and its dispatch's grid: a threadgroup an item (`ph.group`), else a lane an item.
UKL_INLINE void delta_set_grid(UKL_GLOBAL uint32_t* ctl, PhaseP ph, uint32_t n) {
    ctl[DELTA_COUNT_AT + ph.k] = n;
    ctl[DELTA_GRID_AT + 3u * ph.k] = ph.group != 0u ? n : (n + DELTA_TG - 1u) / DELTA_TG;
    ctl[DELTA_GRID_AT + 3u * ph.k + 1u] = 1u;
    ctl[DELTA_GRID_AT + 3u * ph.k + 2u] = 1u;
}

// A phase's count kernel (one thread), its body a function returning its items: the dispatch after it sized by them
// (none once the launch is over), its first turn counted.
#define DELTA_COUNT(name)                                                                                           \
    UKL_INLINE uint32_t name##_n(DELTA_KPARAMS);                                                                    \
    DELTA_ARGS(name, GID0, uint32_t, i, )                                                                           \
    {                                                                                                               \
        if (i != 0u) return;                                                                                        \
        bool on = DELTA_GO && !DELTA_STOPPED;                                                                       \
        if (on) ctl[DELTA_TURNS_AT + ph.k] += 1u;                                                                   \
        delta_set_grid(ctl, ph, on ? name##_n(DELTA_KARGS) : 0u);                                                   \
    }                                                                                                               \
    UKL_INLINE uint32_t name##_n(DELTA_KPARAMS)

// A round phase `name`'s `again` kernel (one thread, after each turn), its body a function returning whether it runs
// another: if so, the next turn's dispatch sized by its count (`name_items`'s) and the turn counted; else none.
#define DELTA_AGAIN(name)                                                                                           \
    UKL_INLINE bool name##_more(DELTA_KPARAMS);                                                                     \
    DELTA_ARGS(name##_again, GID0, uint32_t, i, )                                                                   \
    {                                                                                                               \
        if (i != 0u || !DELTA_GO || DELTA_STOPPED) return;                                                          \
        bool more = name##_more(DELTA_KARGS);                                                                       \
        ctl[DELTA_GO_AT + ph.k] = more ? 1u : 0u;                                                                   \
        if (more) ctl[DELTA_TURNS_AT + ph.k] += 1u;                                                                 \
        delta_set_grid(ctl, ph, more ? name##_items_n(DELTA_KARGS) : 0u);                                           \
    }                                                                                                               \
    UKL_INLINE bool name##_more(DELTA_KPARAMS)

// A phase of lanes: thread `i` item `i`; `DELTA_ITEM(k)` names it, or returns past the phase's items.
#define DELTA_LANES(name) DELTA_ARGS(name, GID0, uint32_t, i, )
#define DELTA_ITEM(k) uint32_t k = i; if (k >= DELTA_N || DELTA_STOPPED) return

// A phase of threadgroups: group `g` item `g`, thread `lid` of `lsz` (`DELTA_TG`), its group memory `sh` beside
// `ukl/group.h`'s `scratch`. `DELTA_GROUP_ITEM(k)` names the item, or returns the whole group past the phase's items
// or once the launch is over (read once, by one thread: a group's threads all return or none).
#define DELTA_GROUPS(name)                                                                                          \
    DELTA_ARGS(name, SHARED, uint32_t, scratch, UKL_TG_SCRATCH, SHARED, uint32_t, sh, DELTA_SHARED, TGID0, uint32_t, \
               g, , LID0, uint32_t, lid, , LSZ0, uint32_t, lsz, )
#define DELTA_GROUP_ITEM(k)                                                                                         \
    if (lid == 0u) scratch[0] = (g >= DELTA_N || DELTA_STOPPED) ? 1u : 0u;                                          \
    ukl_tg_barrier();                                                                                               \
    bool k##_out = scratch[0] != 0u;                                                                                \
    ukl_tg_barrier();                                                                                               \
    if (k##_out) return;                                                                                            \
    uint32_t k = g

// A phase of one item (its count kernel, then the item's lane).
#define DELTA_ONE(name)                                                                                             \
    DELTA_COUNT(name##_items) { return 1u; }                                                                        \
    DELTA_LANES(name)

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
    meter[DELTA_OVER_AT] = 1u;
    return false;
}
#define REACH(k, end) reach(meter, k, end)

// The launch refused, by reason `why` (not 0): it is one its kernels do not do, and the host fails it, naming it. The
// word after the fronts (`meter[3 * MAX_SLOTS]`).
#define FAULT(why) (meter[DELTA_FAULT_AT] = (why))

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
