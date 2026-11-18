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
3. **A scan whose candidates are captured and emitted at once is a segmented pass** (`scan_marks`). This applies
   where a closure's exit, or a `desc`'s next words, is `capture; emit` with nothing captured before.
   - The scan reads a window of 64 nodes at a time. Their flags and tests are read side by side, none waiting on
     another, into two masks: the nodes given (named, where the scan is named) and the nodes entered.
   - A node the scan reaches but does not enter hides its subtree's bits. Its size is read only then, and a subtree
     past the window moves the scan to its end.
   - The window's marks are then a mask, written a word of 32 nodes at a time. A tally by start adds the count
     once, and a tally by node adds each set bit. An emit that marks the start does so at the first node given,
     and so does a pack's emit (its row, its `fired` bit).
   - A `desc` of every node reads nothing: its marks are its range.
   - Before it starts, a scan longer than 4 spill budgets of nodes (2,048 at the budget of 512) is halved into the
     work queue (`scan_cut`, item 2) until its own piece fits. So a long scan's pieces run side by side from the
     start, rather than after `budget` steps one thread at a time.
   - A frame taken from the queue scans the same way. Before, it walked a node at a time.
   - A step is a window or a size read.
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
| The segmented pass, its pieces halved before the scan | 58–71 ms | 25–35 ms | 60.8 M / 4.0 M | 51–94 |

The two kernels overlap on the device, so their sum is the one to compare: 265–340 ms before, and 93–120 ms with the
range marks at load 18–20. The segmented pass ran at load 51–94 only, so its times are high for the load. Its steps
are the like-for-like number. The loop tier's steps fell from 7.1 M to 4.0 M. The longest closure or `desc` walk
fell from 4,106 (loop tier) and 2,149 (t0) to 63, and the longest walk of any kind is 509 (see Open).

Runs: CPU 7.7–8.2 s and wall 2.4–3.0 s at load 18–20, against CPU 8.0–8.2 s and wall 2.8 s at load 16 before.

The spill test (`spilled_walks_find_the_cpus_roots`, a budget of one step, 24 fixtures) took 31 s, against 112 s
with the cut at the candidate's subtree end. At that budget a scan's piece is 128 nodes, so the test halves every
long scan before it starts; it took 41–44 s and found the CPU's roots.

## Open

- The longest walk at s1000 is 509 steps, against Q56's bound of 100: `save0 after kind=238 load0 capture emit0`,
  877,007 walks averaging 12.6 steps. It is a scan of the start's following siblings for one of a kind. Each hop
  over a sibling's subtree reads its size, so the hops wait on each other, and a block of n statements makes n
  such scans, n² hops in all. Neither change here touches `following` (resume 3). Work queue Q65.
