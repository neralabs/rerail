# 0380. A child loop is one scan of the subtree, a long scan is halved, and a captured `desc` marks its range

Status: accepted (2026-11-17)

## Intro

The device's label walks run a small program from each start node: tests, moves (`child`, `desc`, `next`,
`parent`), captures and emits. Work queue Q56 found the loop tier (`label_micro`) the largest walk kernel, at
171–203 ms of GPU time at s1000. Its walks took hundreds of steps one after another, and up to 4,280.

The user asked for two changes:
- a `child` loop as a segmented pass;
- `desc` as a scan of the subtree's range, 64 codes at a time.

In neither case is a list of nodes built first.

## What the time was

internal_dead_code at s1000, every matcher wave pinned to the GPU, `RERAIL_GPU_KERNEL_TIMES=1`
`RERAIL_GPU_WALK_STEPS=1`. A spike kept one loop-tier pass at a time.

| Pass kept | `label_micro` |
|---|---|
| The 49-word pass (no loop) | 3.4–3.5 ms |
| `kind=145 class=16460 closure(class=16407) …` | 34–39 ms |
| `class=16408 … closure(class=16407) … desc capture emit1` | 21–22 ms |
| `class=16393 closure(class=16390) capture emit1` | 173–189 ms |

The last pass's walks total only 0.8 M steps. With spilling off, one of its walks took 136,902 steps alone: a
scan of one large file's subtree, on one thread, which was the dispatch's critical path. Spilling did not help,
because a scan's spill handed on everything after the current candidate's subtree. The thread that took it ran
512 steps and handed on the rest again, so the pieces still ran one after another. Fewer steps did not help either:
the closures below cut the loop tier's steps from 19.0 M to 3.9 M, and `label_micro` stayed at 188–227 ms.

## Decision

1. **A child loop is one move, a closure** (`compile/micro.rs`'s `CLOSURE`, op 0x17).
   - The loop is `i CHILD; i+1 FORK i+3; i+2 GOTO k; i+3..j-1 node tests; j GOTO i`. It is now `CLOSURE j` at `i`.
   - A closure scans the start's subtree in preorder. A node that passes the tests is entered: the scan's next
     node is its first descendant. A node that fails has its subtree skipped (a named closure skips an unnamed
     node's too).
   - Each node the scan reaches continues at `k`.
   - The loop's body words stay in place, so Reach and the label-row remaps read them as before.
   - A walk with a closure runs in the loop tier: its length follows its start's subtree. In the short tier's SIMD
     rounds, closures among walks of a dozen steps took `label_micro_8` from 91–101 to 281–305 ms.
2. **A long scan is halved** (`micro_spill`, resume 4).
   - A closure's frame of at least 128 nodes left to scan spills like a descendant range, by halving.
   - The cut starts at the middle node. It moves past the subtree of the highest ancestor of that node that the
     scan skips, between the scan's next node and the middle node. The ancestors are found by format 16's parent
     climb.
   - Both halves then reach the cut exactly, and each is a scan of its own. So a long scan spreads over the
     dispatch's threads in halvings.
3. **A scan whose candidates are captured and emitted at once marks them in place.** This applies where the
   closure's exit (or a `desc`'s next words) is `capture; emit` with nothing captured before:
   - **The closure** reads a window of 8 nodes' sizes, flags and tests together, and gathers a label's marks a
     word at a time.
   - **`desc`** marks its subtree's range as a range:
     - where every node counts, 32 nodes a word op;
     - for a named `desc`, the named nodes' flags are read one after another, with no frame.
     - A tally by start adds the count once.
     - An emit that marks the start does so at the range's first candidate.
     - This holds in a pack (`fusion::Fused`) too: its emit's row, or the start with its `fired` bit.
4. Nothing is precomputed. Every read is of the tape in place: sizes, flags, kinds and classes. The device's parent
   stays the climb (0359).

## Measured

Same command, findings md5 db93ef6db62fb0961f159c0c667254e1 in every run (unchanged from before).

| Engine | `label_micro_8` | `label_micro` | Walk steps t0 / t2 | Load |
|---|---|---|---|---|
| Before (0359) | 88–113 ms | 177–227 ms | 75.8 M / 19.0 M | 15–17 |
| Closures in the loop tier | 93–99 ms | 196–202 ms | 75.8 M / 3.9 M | 16 |
| Closures in the short tier | 262–277 ms | 1.7 ms | | 16 |
| The halving cut | 92–98 ms | 33 ms | 75.8 M / 7.1 M | 22 |
| A captured `desc` marks its range (labels and tallies) | 73–77 ms | 42–49 ms | 61.8 M / 7.1 M | 73–99 |
| … in packs and start marks too | 45–55 ms | 47–65 ms | 60.9 M / 7.1 M | 18–20 |

The two kernels overlap on the device, so their sum is the one to compare: 265–340 ms before, 93–120 ms after. The
loop tier's steps grew from 3.9 M to 7.1 M with the halving. A spilled piece resumes a frame at a time rather than
through the fused scan, and it counts each node it reaches.

Runs: CPU 7.7–8.2 s and wall 2.4–3.0 s at load 18–20, against CPU 8.0–8.2 s and wall 2.8 s at load 16 before.

The spill test (`spilled_walks_find_the_cpus_roots`, a budget of one step, 24 fixtures) took 31 s, against 112 s
with the cut at the candidate's subtree end.

## Open

- Q56's done-when asks for a longest walk under 100 steps at s1000. The longest is still 4,106 steps (the
  `class=16393` closure, in its pieces' counting). In t0 it is 2,149, against 4,280 before: a named `desc`'s range,
  counted a node read each, though no read waits on the one before. The halving bounds a scan's critical path, not
  its counted steps.
- A spilled closure piece walks its scan a frame at a time. Re-entering the fused scan would cut its steps.
