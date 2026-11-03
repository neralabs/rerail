#include <metal_stdlib>
using namespace metal;

// A device work queue (list step 17, decision 0027): a persistent dispatch's workers take items
// from it and push the items their work creates; idle workers take those up. Prepended to a
// kernel's source (`metal_work_queue.rs`).
//
// The queue is linear, its capacity the most items the work can push (a lattice cell is pushed
// at most once per change of an input), so no slot is used twice and no taker can see a stale
// item. Running out of it is reported (`control[3]`), never wrapped.
//
// No worker waits on another without a bound, as Metal promises no progress between threadgroups
// and none between the lanes of a SIMD group:
// - a slot is handed over by exchange, never awaited. A taker that finds a slot not yet written
//   marks it TAKEN and moves on; the pusher, seeing TAKEN, takes another slot.
// - every loop is bounded (`WQ_TURNS`): the compiler miscompiled an unbounded worker loop (an
//   endless `while` overcounted a thread's own counter; a bounded `for` did not). A bound
//   reached ends the worker, as an idle one does.
// - an idle worker polls a bounded number of times, then exits. A worker that pushed is still
//   running, so a queued item always has a live worker; the host re-launches the dispatch if any
//   remain at its end (`control[2]`).
//
// control: 0 head, 1 tail, 2 active (pushed, not yet done), 3 overflow.

constant uint WQ_EMPTY=0u;
constant uint WQ_TAKEN=0xFFFFFFFFu;
// A worker's turns, and a take's attempts, at most.
constant uint WQ_TURNS=100000000u;

struct WorkQueue { device atomic_uint* slots; device atomic_uint* control; uint capacity; };

// Pushes `item` (below 0xFFFFFFFE). False when the queue is full (the overflow flag is set).
static bool wq_push(WorkQueue q,uint item){
  atomic_fetch_add_explicit(&q.control[2],1u,memory_order_relaxed);
  // Each attempt takes a new index, so the capacity bounds them.
  for(uint attempt=0u;attempt<=q.capacity;attempt++){
    uint tail=atomic_fetch_add_explicit(&q.control[1],1u,memory_order_relaxed);
    if(tail>=q.capacity){
      atomic_store_explicit(&q.control[3],1u,memory_order_relaxed);
      atomic_fetch_sub_explicit(&q.control[2],1u,memory_order_relaxed);
      return false;
    }
    if(atomic_exchange_explicit(&q.slots[tail],item+1u,memory_order_relaxed)!=WQ_TAKEN)return true;
    // A taker came first and gave this index up: take another.
  }
  atomic_store_explicit(&q.control[3],1u,memory_order_relaxed);
  atomic_fetch_sub_explicit(&q.control[2],1u,memory_order_relaxed);
  return false;
}

// Takes an item into `item`; false when there was none to take now.
static bool wq_take(WorkQueue q,thread uint& item){
  uint head=atomic_load_explicit(&q.control[0],memory_order_relaxed);
  bool claimed=false;
  for(uint attempt=0u;attempt<WQ_TURNS&&!claimed;attempt++){
    uint tail=min(atomic_load_explicit(&q.control[1],memory_order_relaxed),q.capacity);
    if(head>=tail)return false;
    claimed=atomic_compare_exchange_weak_explicit(&q.control[0],&head,head+1u,memory_order_relaxed,memory_order_relaxed);
  }
  if(!claimed)return false;
  uint value=atomic_exchange_explicit(&q.slots[head],WQ_TAKEN,memory_order_relaxed);
  if(value==WQ_EMPTY)return false;
  item=value-1u;
  return true;
}

// An item's work is done (after the items it pushed).
static void wq_done(WorkQueue q){atomic_fetch_sub_explicit(&q.control[2],1u,memory_order_relaxed);}

// Whether a worker that found nothing should stop: nothing is active, or it has polled `limit` times.
static bool wq_idle(WorkQueue q,thread uint& polls,uint limit){
  if(atomic_load_explicit(&q.control[2],memory_order_relaxed)==0u)return true;
  return ++polls>=limit;
}
