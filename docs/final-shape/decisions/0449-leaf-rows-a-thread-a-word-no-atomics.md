# 0449. A level's leaf rows are a thread a word, each test's bits stored once

Status: accepted (2026-11-19). Follows 0447; closes Q326.

## Intro

`label_leaves` writes the rows of a level's leaf tests, which its formulas then read (0365 §10). A leaf test is a
node's kind, field or flags, a supertype, or a structural test such as "has a next sibling". After 0447 it was most of
the label passes' GPU time at s1000: 32.2 ms of internal_dead_code's 50.7 ms, in 12 passes of about 2.7 ms. The user:
"go ahead with this: label_leaves".

A pass held 21–23 tests, all of them kind, field or flags (internal_dead_code: 192 kind tests, 54 field tests and 12
flags tests over 12 passes).

The kernel was a threadgroup of 128 threads a file, each thread taking words in turn:

- The largest file's group set the pass's time.
- Every hit was an `atomic_fetch_or` into device memory, although each word was one thread's alone.
- The tests were the inner loop, and a code read (`tape_code`) is two dependent loads: the node's code number, then
  its code. So a node's code was read once a test.

## Decision

1. **A thread a word of the batch's rows**, over every file's region (`FORMULA_REGION`). A thread finds its file by a
   binary search over the files' regions. No file is one group, so no one file holds the pass.
2. **Each test's 32 bits are made in a register and stored once.** The word is the thread's own, so the store is
   plain, with no atomic and no zeroing first.
3. **The nodes are the outer loop:** a node's code is read once for each block of 8 tests, whose bits are 8 registers.

## Measured

s1000, pinned to the GPU, the memory line lifted, alternating runs against the engine before (load 34–52). The
first run after each Metal change was discarded. Rows on Metal: internal_dead_code 988d8fbe, public_api_triples
3ffb9c9f, as before.

| | Leaves passes (encoders), before | after | The command buffers' GPU time, before | after |
|---|---|---|---|---|
| internal_dead_code | 31.5–39.6 ms | 7.8–8.0 ms | 55.5–59.5 ms | 42.0–45.3 ms |
| public_api_triples | 26.8–40.3 ms | 7.7–7.9 ms | 44.4–63.3 ms | 36.2–40.9 ms |

One internal_dead_code run after the change split its levels differently (18 leaves passes, 14.5 ms, 59.4 ms in all),
and is left out of the ranges. CPU did not move.

Steps 1 and 2 alone (codes still read once a test) took the leaves passes to 13.2–19.3 ms.

### The formulas run slower after a shorter leaves pass, and the GPU's clock is why

With the change, `label_formula`'s own time went up: 17–20 ms to 28–29 ms in encoder times, and 21.3–28.3 ms to
38.3–42.9 ms with each kernel committed alone (`RERAIL_GPU_KERNEL_TIMES`). Its work did not change: its members,
parent reads, child reads and groups are identical.

Repeating the new leaves pass, a temporary switch that was not kept, gives the same rows and the same cache state,
with more GPU work before the formulas. The formulas got faster:

- once: 34.6–38.0 ms;
- 5 times: 23.9, 26.4 and 36.0 ms;
- 10 times: 21.6 ms.

So the formulas are latency-bound, and they run faster on a GPU that was busy just before. The old leaves pass's
3 ms of work a level kept the GPU's clock up. The command buffers' total is the measure of this change, and it fell.

## What is left

- `label_formula` is now the largest kernel, and its time follows the GPU's clock: it does little work for its
  latency.
