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
// - a worker that finds nothing to take ends at once; it never polls for others' pushes (decision 0540 step 7, the
//   user's: a worker does the work there is, and work that needs another's result is a continuation its finisher
//   pushes). A worker that pushed is still running and takes on, so a queued item has a live worker; a dispatch of
//   more workers than run at once has later ones start as earlier ones end, and take what is queued then. The host
//   launches the dispatch again while any remain at its end (`control[2]`), where a budget ended workers.
// - a worker takes at most `budget` items in a launch (decision 0540), more than an even share where it finishes its
//   items sooner, then ends, as an idle one does (`ukl_wq_spent`): a launch's GPU time is so bounded by its workers'
//   budgets, not by the work. The host launches again while items are left. A launch that leaves any had a worker
//   that spent its budget, so the launches are at most the capacity over the budget, and one more.
// - the control words are read by polls (`ukl_atomic_poll`, decision 0540): on Apple's GPUs a relaxed load in a loop
//   may keep reading the core's copy (Q123), and a worker would see the queue empty, or its work active, for ever.
//
// `control`: 0 head, 1 tail, 2 active (pushed, not yet done), 3 overflow. Items are below 0xFFFFFFFE.
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
    // A worker's items in a launch at most (a SIMD group's takes, for `ukl_wq_take_sg`); 0, the capacity's.
    uint32_t budget;
} UklWorkQueue;

// Pushes `item`. False when the queue is full (the overflow flag is set).
UKL_INLINE bool ukl_wq_push(UklWorkQueue q, uint32_t item)
{
    ukl_atomic_add(&q.control[2], 1u);
    // Each attempt takes a new index, so the capacity bounds them.
    for (uint32_t attempt = 0u; attempt <= q.capacity; attempt++) {
        uint32_t tail = ukl_atomic_add(&q.control[1], 1u);
        if (tail >= q.capacity)
            break;
        if (ukl_atomic_exchange(&q.slots[tail], item + 1u) != UKL_WQ_TAKEN)
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
        uint32_t tail = ukl_atomic_poll(&q.control[1]);
        tail = tail < q.capacity ? tail : q.capacity;
        if (head >= tail)
            return false;
        uint32_t seen = ukl_atomic_cas(&q.control[0], head, head + 1u);
        claimed = seen == head;
        head = claimed ? head : seen;
    }
    if (!claimed)
        return false;
    uint32_t value = ukl_atomic_exchange(&q.slots[head], UKL_WQ_TAKEN);
    if (value == UKL_WQ_EMPTY)
        return false;
    *item = value - 1u;
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
            uint32_t tail = ukl_atomic_poll(&q.control[1]);
            tail = tail < q.capacity ? tail : q.capacity;
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
    uint32_t value = ukl_atomic_exchange(&q.slots[head + lane], UKL_WQ_TAKEN);
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
