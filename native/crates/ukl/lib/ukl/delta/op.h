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

// A wide launch: several instances of one operation, each its own slots (`ph.ns` a one), params (`ph.ps` bytes a one)
// and state, its meter then its control words (`DELTA_STATE_WORDS` a one, `meter` the first's, `ctl` its control).
// Each phase's dispatch is their grids end to end: past every instance's state, the phase's whole grid and the prefix
// of the instances' grids (`DELTA_WIDE_WORDS` a phase), by which an item finds its instance and its place in it, and
// the kernel's arguments are moved to that instance's (`DELTA_ITEM`, `DELTA_GROUP_ITEM`). A count kernel and an
// `again` kernel are one thread, over every instance in turn. One instance is a launch of one.
#define DELTA_MAX_WIDE 32u
#define DELTA_METER_WORDS (3u * MAX_SLOTS + 2u)
#define DELTA_CTL_AT ((DELTA_METER_WORDS + 15u) & ~15u)
#define DELTA_STATE_WORDS (DELTA_CTL_AT + DELTA_CTL_WORDS)
#define DELTA_WIDE_WORDS (4u + DELTA_MAX_WIDE)

// Whether an instance's launch is over: refused, or a slot found too small (it runs again, grown: no phase after does
// work).
UKL_INLINE bool delta_stopped(UKL_GLOBAL const uint32_t* meter) { return meter[DELTA_FAULT_AT] != 0u || meter[DELTA_OVER_AT] != 0u; }
#define DELTA_STOPPED delta_stopped(meter)
// This phase's items, and whether it runs (a round phase stops after its last turn).
#define DELTA_N ctl[DELTA_COUNT_AT + ph.k]
#define DELTA_GO (ctl[DELTA_GO_AT + ph.k] != 0u)

// The arguments every kernel takes, as a function of the kernel's takes them.
#define DELTA_KPARAMS                                                                                               \
    UKL_GLOBAL const uint8_t *t, UKL_GLOBAL const uint32_t *slots, UKL_GLOBAL const uint8_t *p, PhaseP ph,          \
        UKL_GLOBAL uint32_t *ctl, UKL_GLOBAL uint32_t *meter
#define DELTA_KARGS t, slots, p, ph, ctl, meter
// Instance `j`'s arguments.
#define DELTA_KARGS_OF(j) t, slots + (j) * ph.ns, p + (j) * ph.ps, ph, ctl + (j) * DELTA_STATE_WORDS, meter + (j) * DELTA_STATE_WORDS
// The kernel's arguments moved to instance `j`'s.
#define DELTA_MOVE_TO(j)                                                                                            \
    slots += (j) * ph.ns;                                                                                           \
    p += (j) * ph.ps;                                                                                               \
    ctl += (j) * DELTA_STATE_WORDS;                                                                                 \
    meter += (j) * DELTA_STATE_WORDS

// Phase `ph`'s items, `n`, and its dispatch's grid: a threadgroup an item (`ph.group`), else a lane an item.
UKL_INLINE void delta_set_grid(UKL_GLOBAL uint32_t* ctl, PhaseP ph, uint32_t n) {
    ctl[DELTA_COUNT_AT + ph.k] = n;
    ctl[DELTA_GRID_AT + 3u * ph.k] = ph.group != 0u ? n : (n + DELTA_TG - 1u) / DELTA_TG;
    ctl[DELTA_GRID_AT + 3u * ph.k + 1u] = 1u;
    ctl[DELTA_GRID_AT + 3u * ph.k + 2u] = 1u;
}

// Phase `ph`'s words past the instances' state: its whole grid (three words), then the prefix of its instances' grids,
// in threadgroups.
UKL_INLINE UKL_GLOBAL uint32_t* delta_wide(UKL_GLOBAL uint32_t* meter, PhaseP ph) { return meter + ph.w * DELTA_STATE_WORDS + ph.k * DELTA_WIDE_WORDS; }

// The phase's whole grid, from each instance's (`meter` the first's).
UKL_INLINE void delta_wide_grid(UKL_GLOBAL uint32_t* meter, PhaseP ph) {
    UKL_GLOBAL uint32_t* w = delta_wide(meter, ph);
    uint32_t run = 0u;
    for (uint32_t j = 0u; j < ph.w; j++) {
        w[3u + j] = run;
        run += meter[j * DELTA_STATE_WORDS + DELTA_CTL_AT + DELTA_GRID_AT + 3u * ph.k];
    }
    w[3u + ph.w] = run;
    w[0] = run;
    w[1] = 1u;
    w[2] = 1u;
}

// The instance whose part of the phase's grid holds threadgroup `g`, and its first threadgroup into `first`: NONE past
// the grid (a backend may launch more).
UKL_INLINE uint32_t delta_instance(UKL_GLOBAL uint32_t* meter, PhaseP ph, uint32_t g, UKL_PRIVATE uint32_t* first) {
    UKL_GLOBAL const uint32_t* pre = delta_wide(meter, ph) + 3u;
    if (g >= pre[ph.w]) return NONE;
    uint32_t j = 0u;
    while (pre[j + 1u] <= g) j++;
    *first = pre[j];
    return j;
}

// A phase's count kernel (one thread), its body a function returning its items: each instance's dispatch sized by
// them (none once its launch is over), its first turn counted; then the phase's whole grid.
#define DELTA_COUNT(name)                                                                                           \
    UKL_INLINE uint32_t name##_n(DELTA_KPARAMS);                                                                    \
    DELTA_ARGS(name, GID0, uint32_t, i, )                                                                           \
    {                                                                                                               \
        if (i != 0u) return;                                                                                        \
        for (uint32_t j = 0u; j < ph.w; j++) {                                                                      \
            UKL_GLOBAL uint32_t *c = ctl + j * DELTA_STATE_WORDS, *m = meter + j * DELTA_STATE_WORDS;               \
            bool on = c[DELTA_GO_AT + ph.k] != 0u && !delta_stopped(m);                                             \
            if (on) c[DELTA_TURNS_AT + ph.k] += 1u;                                                                 \
            delta_set_grid(c, ph, on ? name##_n(DELTA_KARGS_OF(j)) : 0u);                                           \
        }                                                                                                           \
        delta_wide_grid(meter, ph);                                                                                 \
    }                                                                                                               \
    UKL_INLINE uint32_t name##_n(DELTA_KPARAMS)

// A round phase `name`'s `again` kernel (one thread, after each turn), its body a function returning whether it runs
// another: for each instance still going, if so, its next turn's dispatch sized by its count (`name_items`'s) and the
// turn counted; else none. Then the phase's whole grid.
#define DELTA_AGAIN(name)                                                                                           \
    UKL_INLINE bool name##_more(DELTA_KPARAMS);                                                                     \
    DELTA_ARGS(name##_again, GID0, uint32_t, i, )                                                                   \
    {                                                                                                               \
        if (i != 0u) return;                                                                                        \
        for (uint32_t j = 0u; j < ph.w; j++) {                                                                      \
            UKL_GLOBAL uint32_t *c = ctl + j * DELTA_STATE_WORDS, *m = meter + j * DELTA_STATE_WORDS;               \
            if (c[DELTA_GO_AT + ph.k] == 0u || delta_stopped(m)) {                                                  \
                delta_set_grid(c, ph, 0u);                                                                          \
                continue;                                                                                           \
            }                                                                                                       \
            bool more = name##_more(DELTA_KARGS_OF(j));                                                             \
            c[DELTA_GO_AT + ph.k] = more ? 1u : 0u;                                                                 \
            if (more) c[DELTA_TURNS_AT + ph.k] += 1u;                                                               \
            delta_set_grid(c, ph, more ? name##_items_n(DELTA_KARGS_OF(j)) : 0u);                                   \
        }                                                                                                           \
        delta_wide_grid(meter, ph);                                                                                 \
    }                                                                                                               \
    UKL_INLINE bool name##_more(DELTA_KPARAMS)

// A phase of lanes: each thread an item of its instance; `DELTA_ITEM(k)` names it, the arguments moved to its
// instance's, or returns past the phase's items.
#define DELTA_LANES(name) DELTA_ARGS(name, GID0, uint32_t, i, )
#define DELTA_ITEM(k)                                                                                               \
    uint32_t k##_first = 0u, k##_of = delta_instance(meter, ph, i / DELTA_TG, &k##_first);                         \
    if (k##_of == NONE) return;                                                                                     \
    DELTA_MOVE_TO(k##_of);                                                                                          \
    uint32_t k = i - k##_first * DELTA_TG;                                                                          \
    if (k >= DELTA_N || DELTA_STOPPED) return

// A phase of threadgroups: each group an item of its instance, thread `lid` of `lsz` (`DELTA_TG`), its group memory
// `sh` beside `ukl/group.h`'s `scratch`. `DELTA_GROUP_ITEM(k)` names the item, the arguments moved to its instance's,
// or returns the whole group past the phase's items or once the instance's launch is over (read once, by one thread:
// a group's threads all return or none).
#define DELTA_GROUPS(name)                                                                                          \
    DELTA_ARGS(name, SHARED, uint32_t, scratch, UKL_TG_SCRATCH, SHARED, uint32_t, sh, DELTA_SHARED, TGID0, uint32_t, \
               g, , LID0, uint32_t, lid, , LSZ0, uint32_t, lsz, )
#define DELTA_GROUP_ITEM(k)                                                                                         \
    uint32_t k##_first = 0u, k##_of = delta_instance(meter, ph, g, &k##_first);                                     \
    if (k##_of == NONE) return;                                                                                     \
    DELTA_MOVE_TO(k##_of);                                                                                          \
    uint32_t k = g - k##_first;                                                                                     \
    if (lid == 0u) scratch[0] = (k >= DELTA_N || DELTA_STOPPED) ? 1u : 0u;                                          \
    ukl_tg_barrier();                                                                                               \
    bool k##_out = scratch[0] != 0u;                                                                                \
    ukl_tg_barrier();                                                                                               \
    if (k##_out) return

// A phase of one item (its count kernel, then the item's lane).
#define DELTA_ONE(name)                                                                                             \
    DELTA_COUNT(name##_items) { return 1u; }                                                                        \
    DELTA_LANES(name)

// Block `id`'s bytes, and slot `k`'s.
UKL_INLINE Blk blk(CLIF_BLOCK_SPACE const uint8_t* t, uint32_t id) { return (Blk)desc_row(t, id)->addr; }

// The executor's validation mode: the word a launch's kernels count their state reads into (its `kind::WORK` block's
// body, named by the table's last row, `DESC_WORK_ROW`), or none (0) when the launch is not validated: one load.
UKL_INLINE CLIF_BLOCK_SPACE uint32_t* delta_work(CLIF_BLOCK_SPACE const uint8_t* t) {
    uint64_t n = *(CLIF_BLOCK_SPACE const uint64_t*)t;
    CLIF_BLOCK_SPACE const DescRow* last = (CLIF_BLOCK_SPACE const DescRow*)(t + 8) + (n - 1u);
    return n != 0u && last->id == DESC_WORK_ROW ? (CLIF_BLOCK_SPACE uint32_t*)(last->addr + BODY_AT) : 0;
}

// `n` state reads counted, where the launch counts them (the twins' `State::touch`).
UKL_INLINE void delta_touch(CLIF_BLOCK_SPACE uint32_t* work, uint32_t n) {
    if (work != 0) ukl_atomic_add((UKL_GLOBAL uint32_t*)work, n);
}
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
