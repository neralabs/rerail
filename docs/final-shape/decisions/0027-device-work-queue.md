# 0027: device-side enqueue is a work queue inside a persistent dispatch

Status: accepted (2026-11-02, list step 17; reworked the same day at the user's direction, replacing
a first version that limited device loops to one threadgroup's own loop)

## Context

Phase 4 (e) asks for device-side enqueue of in-file loops whose bound is not fixed (E9), "from G
indirect dispatch to indirect command buffers"; (f) asks for file-local lattice FIX waves (I10, R0).

What Metal offers:
- A kernel cannot launch a kernel.
- An indirect command buffer (ICB) can be encoded by the GPU, but only into commands the host
  reserved, so on its own it is a fixed bound with early exit, the shape G's indirect dispatch
  already has.
- A dispatch's threads can share an atomic work queue in device memory: workers take items, push
  the items their work creates, and idle workers take those up, across every threadgroup. That
  is device-side enqueue: the device decides what runs next and when the loop ends. Device-scope
  fences and `coherent(device)` buffers are available (checked on this machine, an M5 Max).
- Metal promises no forward progress between threadgroups, nor between the lanes of one SIMD
  group, so a worker must never wait on another without a bound.

The evidence:
- ast-index-spike: GPU work is absent (DI §19, "Nothing was designed or measured for GPU"), so
  this is new ground. The spike's structural acceptance rule applies: a bound must follow from the
  work, not be a fixed budget (lessons ledger row 26).
- Tempo-metal's host-driven generations stopped at 595 dispatches (00-review), and one task row
  per tuple needed 1.99 GB (ledger row 1): work must not be planned per round by the host, nor
  stored per tuple.

## Decision

A loop the device decides runs on the **device work queue** (`metal_work_queue.{metal,rs}`):

- **One persistent dispatch**, sized to what the device keeps resident (at most 64 groups of 256
  workers).
- **A linear queue.** Its capacity is the most items the work can push, so no slot is reused and
  no taker can see a stale item. Running out is an error, never a wrap.
- **No unbounded wait.** A slot is handed over by exchange: a taker that finds it unwritten marks
  it TAKEN and moves on, and the pusher takes another slot. An idle worker polls a bounded number
  of times, then exits. A worker that pushed is still running, so a queued item always has a
  live worker.
- **Re-launch as the safety net.** If items remain once every worker has exited, the host
  launches the same dispatch again over the same queue (at most 8 launches; more is a failure).
  This is not a planned round.

Its first user is **file-local lattice FIX** (`metal_lattice::solve`, kernel `lattice_queue`):
- An item is a cell to evaluate. A cell whose value rose pushes its dependents, de-duplicated by
  a queued bit.
- The order is chaotic; the system is monotone over a finite lattice, so it reaches the one least
  fixpoint. The CPU's `lattice_fixpoint` is the reference.
- A cell's state only rises: BOT, writing, constant, TOP. A constant is published by a claim, a
  write, a fence and a release. A reader that sees "writing" reads BOT (the writer pushes the
  dependents afterwards), and a raise that finds "writing" pushes its cell again instead of
  waiting. No value falls, and none is read half written.
- `logic::native::lattice_fixpoint(..., 'metal')` binds it for a flat lattice without widening.

Label groups keep `Runner::group` (a file per threadgroup, its rounds to the fixpoint in one
dispatch). Their rounds evaluate complements and counts between barriers, so they are stratified
rather than monotone, and are not a queue's push-on-change work. Spreading whole files across
workers would only redo the hardware's own threadgroup scheduling.

`R[tid]` (g) needs no loop. A walk's text slot is read from the term index (`prefilter::terms`).

## Not decided here

- The queue kernel runs only the code compiled into it. A recursive group whose members are
  interpreter tasks or per-clause pattern kernels (other pipelines) keeps host-planned rounds
  with device early exit until those members run in a queue kernel (O34). G's grid executor's
  frontier waves stay as O33.
- Widened and non-flat lattices stay on the CPU.
- Which files' lattices go to the device (by size) is not decided.
