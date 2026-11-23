// ukl/workqueue.h: a device work queue for a persistent dispatch (decisions 0440 §5, 0027: the protocol of the engine's
// Metal queue, `metal_work_queue.metal`, which this replaced, on any backend).
//
// A persistent dispatch's workers take items and push the items their work creates; idle workers take those up.
// The queue is linear: its capacity is the most items the work can push, so no slot is used twice and no taker can
// see a stale item. Running out of it is reported (`control[3]`), never wrapped.
//
// No worker waits on another without a bound (Metal promises no progress between threadgroups, nor between the lanes
// of a SIMD group):
// - a slot is handed over by exchange, never awaited: a taker that finds a slot not yet written marks it TAKEN and
//   moves on; the pusher, seeing TAKEN, takes another slot;
// - every loop is bounded (`UKL_WQ_TURNS`); a bound reached ends the worker, as an idle one does;
// - a worker that finds nothing to take ends at once (decision 0540 step 7, the user's: a worker does the work there
//   is, and work that needs another's result is a continuation its finisher pushes). A worker that pushed is still
//   running and takes on, so a queued item has a live worker; a dispatch of more workers than run at once has later
//   ones start as earlier ones end, and take what is queued then. The host launches the dispatch again while any
//   remain at its end (`control[2]`), where a budget ended workers.
// - except where a whole step's items are pushed only once the step before is done (a program's phases, decision
//   0637): there the finisher would take them alone, every other worker ended. An idle worker of such a dispatch polls
//   (`ukl_wq_ready`), a bounded number of times, and takes what is written (`ukl_wq_take_ready`). No worker waits for
//   it: a poller that never runs, or ends, only leaves its items to the others.
// - a worker takes at most `budget` items in a launch (decision 0540), more than an even share where it finishes its
//   items sooner, then ends, as an idle one does (`ukl_wq_spent`): a launch's GPU time is so bounded by its workers'
//   budgets, not by the work. The host launches again while items are left. A launch that leaves any had a worker
//   that spent its budget, so the launches are at most the capacity over the budget, and one more.
// - the control words are read by polls (`ukl_atomic_poll`, decision 0540): on Apple's GPUs a relaxed load in a loop
//   may keep reading the core's copy (Q123), and a worker would see the queue empty, or its work active, for ever.
//
// `control`: 0 head, 1 tail, 2 active (pushed, not yet done), 3 overflow, 4 a ring's base. Items are below 0xFFFFFFFE.
//
// A ring (`ring` set, decision 0486 §10): positions run on past the capacity, position `p` in slot `p % capacity`,
// and a push takes positions below `base + capacity` alone. At a pause, when no worker runs (so no taker is between
// its claim of a position and its exchange of the slot), `ukl_wq_rebase_clear` and `ukl_wq_rebase` move the base up
// to the head: the slots of the positions taken since are emptied, and pushes may use them again. Every slot a push
// then reuses held a position below the head, taken and exchanged before the pause, so no taker sees another
// position's item. The queue's room is then the most items queued between two pauses, not all items ever pushed.
//
// Two ways to take: a thread at a time (`ukl_wq_take`), or a SIMD group at a time (`ukl_wq_take_sg`, the group's lanes
// ending together when none took any: `ukl_sg_count`), where one lane claims up to a group's worth of items with one compare-and-swap and hands them
// out. Every taker contends for the head: a thread at a time, thousands of workers make each item cost thousands of
// failed swaps where atomics are slow (measured on a GB10, whose blocks are pinned host memory: 1.5 ms an item with
// 4,096 workers). A group at a time, the contenders and the swaps are `UKL_SIMD_WIDTH` times fewer.

#pragma once
#include <ukl/simd.h>

#define UKL_WQ_EMPTY 0u
#define UKL_WQ_TAKEN 0xFFFFFFFFu
// A take's attempts at most.
#define UKL_WQ_TURNS 100000000u

typedef struct UklWorkQueue {
    UKL_GLOBAL uint32_t* slots;
    UKL_GLOBAL uint32_t* control;
    uint32_t capacity;
    // A ring (1, with `control[4]`) or linear (0).
    uint32_t ring;
    // A worker's items in a launch at most (a SIMD group's takes, for `ukl_wq_take_sg`); 0, the capacity's.
    uint32_t budget;
} UklWorkQueue;

// The positions' end: a push takes positions below it.
UKL_INLINE uint32_t ukl_wq_end(UklWorkQueue q) { return q.ring != 0u ? ukl_atomic_poll(&q.control[4]) + q.capacity : q.capacity; }

// Position `p`'s slot.
UKL_INLINE uint32_t ukl_wq_slot(UklWorkQueue q, uint32_t p) { return q.ring != 0u ? p % q.capacity : p; }

// Pushes `item`. False when the queue is full (the overflow flag is set).
UKL_INLINE bool ukl_wq_push(UklWorkQueue q, uint32_t item)
{
    ukl_atomic_add(&q.control[2], 1u);
    uint32_t end = ukl_wq_end(q);
    // Each attempt takes a new index, so the capacity bounds them.
    for (uint32_t attempt = 0u; attempt <= q.capacity; attempt++) {
        uint32_t tail = ukl_atomic_add(&q.control[1], 1u);
        if (tail >= end)
            break;
        if (ukl_atomic_exchange(&q.slots[ukl_wq_slot(q, tail)], item + 1u) != UKL_WQ_TAKEN)
            return true;
        // A taker came first and gave this index up: take another.
    }
    ukl_atomic_store(&q.control[3], 1u);
    ukl_atomic_sub(&q.control[2], 1u);
    return false;
}

// Takes an item into `item`; false when there was none to take now.
UKL_INLINE bool ukl_wq_take(UklWorkQueue q, UKL_PRIVATE uint32_t* item)
{
    uint32_t head = ukl_atomic_poll(&q.control[0]);
    bool claimed = false;
    for (uint32_t attempt = 0u; attempt < UKL_WQ_TURNS && !claimed; attempt++) {
        uint32_t tail = ukl_atomic_poll(&q.control[1]), end = ukl_wq_end(q);
        tail = tail < end ? tail : end;
        if (head >= tail)
            return false;
        uint32_t seen = ukl_atomic_cas(&q.control[0], head, head + 1u);
        claimed = seen == head;
        head = claimed ? head : seen;
    }
    if (!claimed)
        return false;
    uint32_t value = ukl_atomic_exchange(&q.slots[ukl_wq_slot(q, head)], UKL_WQ_TAKEN);
    if (value == UKL_WQ_EMPTY)
        return false;
    *item = value - 1u;
    return true;
}

// Whether the item at the head is written: a poller's check before it takes (decision 0637), by polls alone.
UKL_INLINE bool ukl_wq_ready(UklWorkQueue q)
{
    uint32_t head = ukl_atomic_poll(&q.control[0]), tail = ukl_atomic_poll(&q.control[1]), end = ukl_wq_end(q);
    tail = tail < end ? tail : end;
    if (head >= tail)
        return false;
    uint32_t v = ukl_atomic_poll(&q.slots[ukl_wq_slot(q, head)]);
    return v != UKL_WQ_EMPTY && v != UKL_WQ_TAKEN;
}

// A poller's take (decision 0637): the item at the head where it is written, else none, and no slot given up. A
// worker that polls and took with `ukl_wq_take` would give up a slot at each poll that met a push between its tail
// and its write, and use the queue's room. Only takers claim the head, and each exchanges its own position's slot,
// so a written slot seen at the head is the claimer's item.
UKL_INLINE bool ukl_wq_take_ready(UklWorkQueue q, UKL_PRIVATE uint32_t* item)
{
    uint32_t head = ukl_atomic_poll(&q.control[0]), tail = ukl_atomic_poll(&q.control[1]), end = ukl_wq_end(q);
    tail = tail < end ? tail : end;
    if (head >= tail)
        return false;
    uint32_t slot = ukl_wq_slot(q, head), v = ukl_atomic_poll(&q.slots[slot]);
    if (v == UKL_WQ_EMPTY || v == UKL_WQ_TAKEN || ukl_atomic_cas(&q.control[0], head, head + 1u) != head)
        return false;
    *item = ukl_atomic_exchange(&q.slots[slot], UKL_WQ_TAKEN) - 1u;
    return true;
}

// A SIMD group's take: lane `lane` gets an item into `item` (true) or none. Convergent: every lane calls it.
UKL_INLINE bool ukl_wq_take_sg(UklWorkQueue q, uint32_t lane, UKL_PRIVATE uint32_t* item __UKL_PARAMS)
{
    // Lane 0 claims `count` items from `head` at once.
    uint32_t head = 0u, count = 0u;
    if (lane == 0u) {
        head = ukl_atomic_poll(&q.control[0]);
        for (uint32_t attempt = 0u; attempt < UKL_WQ_TURNS; attempt++) {
            uint32_t tail = ukl_atomic_poll(&q.control[1]), end = ukl_wq_end(q);
            tail = tail < end ? tail : end;
            if (head >= tail)
                break;
            uint32_t want = tail - head < UKL_SIMD_WIDTH ? tail - head : UKL_SIMD_WIDTH;
            uint32_t seen = ukl_atomic_cas(&q.control[0], head, head + want);
            if (seen == head) {
                count = want;
                break;
            }
            head = seen;
        }
    }
    head = ukl_sg_broadcast(head, 0u __UKL_ARGS);
    count = ukl_sg_broadcast(count, 0u __UKL_ARGS);
    if (lane >= count)
        return false;
    uint32_t value = ukl_atomic_exchange(&q.slots[ukl_wq_slot(q, head + lane)], UKL_WQ_TAKEN);
    if (value == UKL_WQ_EMPTY)
        return false;
    *item = value - 1u;
    return true;
}

// Whether a worker that has taken `taken` items in this launch (a SIMD group: its takes that got any) has spent its
// budget: it ends, and the items left are the next launch's.
UKL_INLINE bool ukl_wq_spent(UklWorkQueue q, uint32_t taken) { return q.budget != 0u && taken >= q.budget; }

// An item's work is done (after the items it pushed).
UKL_INLINE void ukl_wq_done(UklWorkQueue q) { ukl_atomic_sub(&q.control[2], 1u); }

// A ring's pause, first step (every thread, `k` from 0 below `ukl_wq_taken(q)`): the slot of the `k`th position taken
// since the base emptied.
UKL_INLINE uint32_t ukl_wq_taken(UklWorkQueue q) { return ukl_atomic_poll(&q.control[0]) - ukl_atomic_poll(&q.control[4]); }
UKL_INLINE void ukl_wq_rebase_clear(UklWorkQueue q, uint32_t k)
{
    ukl_atomic_store(&q.slots[ukl_wq_slot(q, ukl_atomic_poll(&q.control[4]) + k)], UKL_WQ_EMPTY);
}

// A ring's pause, second step (one thread, after a barrier): the base at the head.
UKL_INLINE void ukl_wq_rebase(UklWorkQueue q) { ukl_atomic_store(&q.control[4], ukl_atomic_poll(&q.control[0])); }

// The positions queued past the base: the room a ring's run used (past `capacity`: it ran out).
UKL_INLINE uint32_t ukl_wq_used(UklWorkQueue q) { return ukl_atomic_poll(&q.control[1]) - ukl_atomic_poll(&q.control[4]); }
