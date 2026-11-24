# 0380. A child loop is one scan of the subtree, a long scan is halved, and a captured `desc` marks its range

Status: built (audited 2026-11-23). Accepted (2026-11-17)

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
3. **A move's candidates are one segmented pass wherever the words to the emit read the candidate alone**
   (`scan_marks`). The rule matches the walk's shape, not a pass or a rule.
   - **The moves:** every move with a frame of candidates. `desc` is a range, a closure a range with skips, and
     `child` and `after` a chain of siblings up to the parent's end.
   - **The tail:** the words from the candidate to its emit (`ScanTail`) are tests of the candidate, then at most
     one `load` of a saved node and tests of that node, at most one capture, and the emit. Anything else (another
     move, a fork, a save) keeps the walk a frame at a time.
   - **The windows:** the scan reads 64 nodes at a time. A window's codes and slots lie in a line or two of the
     tape, so its reads go out together and none waits on a far one:
     - a range's candidates are its nodes;
     - a closure's are those its skips leave: a node reached but not entered hides its subtree's bits, and its size
       is read only where it fails;
     - a chain's are its hops over each sibling's subtree, inside the window's slots, moving to the next window
       where a hop leaves it.
   - **The outputs:** a named move drops the unnamed candidates, and the tail's tests drop the failing ones, leaving
     a mask of outputs.
     - Where each output marks its own node (the captured candidate), the marks are the mask, a word of 32 nodes at
       a time. Tallies add a count.
     - Where one output makes every mark (the start, or a saved node, as in `after kind=K load0 capture emit`), the
       scan stops at its first output.
   - **Splitting:** where every output counts, a scan longer than 4 spill budgets of nodes (2,048 at the budget of
     512) is halved into the work queue before it starts (`scan_cut`, item 2), until its own piece fits. A chain is
     cut at the sibling that holds the middle node, found by a climb to the parent. A frame taken from the queue
     scans the same way.
   - A step is a window.
4. Nothing is precomputed. Every read is of the tape in place: sizes, flags, kinds and classes. The device's parent
   stays the climb (0359).

## Measured

Same command. These runs were checked by a findings md5, which was the md5 of a constant: the program finds
nothing at s1000 (0390). The check that holds is each output relation's rows, which 0390's runs compare.

| Engine | `label_micro_8` | `label_micro` | Walk steps t0 / t2 | Load |
|---|---|---|---|---|
| Before (0359) | 88–113 ms | 177–227 ms | 75.8 M / 19.0 M | 15–17 |
| Closures in the loop tier | 93–99 ms | 196–202 ms | 75.8 M / 3.9 M | 16 |
| Closures in the short tier | 262–277 ms | 1.7 ms | | 16 |
| The halving cut | 92–98 ms | 33 ms | 75.8 M / 7.1 M | 22 |
| A captured `desc` marks its range (labels and tallies) | 73–77 ms | 42–49 ms | 61.8 M / 7.1 M | 73–99 |
| … in packs and start marks too | 45–55 ms | 47–65 ms | 60.9 M / 7.1 M | 18–20 |
| The segmented pass, its pieces halved before the scan | 58–71 ms | 25–35 ms | 60.8 M / 4.0 M | 51–94 |
| Every move by its tail's shape (`child`, `after` too) | 43.5–46.8 ms | 24.6–33.1 ms | 59.7 M / 4.0 M | 13–14 |

The two kernels overlap on the device, so their sum is the one to compare: 265–340 ms before, and 93–120 ms with the
range marks at load 18–20. The segmented pass ran at load 51–94 only, so its times are high for the load. Its steps
are the like-for-like number. The loop tier's steps fell from 7.1 M to 4.0 M. The longest closure or `desc` walk
fell from 4,106 (loop tier) and 2,149 (t0) to 63, and the longest walk of any kind is 509 (see Open).

Runs: CPU 7.7–8.2 s and wall 2.4–3.0 s at load 18–20, against CPU 8.0–8.2 s and wall 2.8 s at load 16 before.

The spill test (`spilled_walks_find_the_cpus_roots`, a budget of one step, 24 fixtures) took 31 s, against 112 s
with the cut at the candidate's subtree end. At that budget a scan's piece is 128 nodes, so the test halves every
long scan before it starts; it took 41–44 s and found the CPU's roots.

## Q182: the `after` scan by windows

The longest walk left after the range scans was `save0 after kind=238 load0 capture emit0`: 877,007 walks of 12.6
steps on average, the longest 509. It scans the start's following siblings for one of kind 238. Under item 3 its
hops run inside windows of slots, and it stops at its first output. Its longest walk fell to 290 steps, and the
longest walk at s1000 is now 290.

Splitting the scan wanted once was tried and is not kept. Halving it after a piece of its own found nothing brought
the longest walk to 74, under Q56's bound of 100, but it cost time. Five pairs of runs alternated at load 13–29:

| Scans wanted once | `label_micro_8` | `label_micro` | Longest walk |
|---|---|---|---|
| Whole, stopping at the first output (kept) | 41.5–56.3 ms | 25.4–42.5 ms | 290 |
| Halved after a first piece | 83.0–108.3 ms | 11.9–16.4 ms | 74 |

The loss is the queue, not the cut or the scan:
- The halved runs walked the same steps, to within 0.003%. A check of the mark between windows, so a piece stops
  once another piece has marked it, saved nothing.
- Handing on the rest whole, with no climb, took 113–128 ms.
- Moving the `after` walks to the loop tier, so their pieces run there, totalled 103–119 ms.
- A thread taking one queued piece between its items took 101–105 ms.

In the short tier a given piece waits for the dispatch's tail, and its SIMD group's lanes wait for the lane that
cuts. The step bound stood in for a dispatch's critical path. Here the dispatch's time and the bound disagree, so
the time decides.

## Q182: one pass over a parent's children, its answer shared by the siblings

The user asked for this to be built, generic over its shape (2026-11-17). `sibling_pass` matches by shape only:
- **The move:** an `after` scan that is the walk's first move, reached from its start through saves, forks and
  jumps only. So every start's walk reaches it with the start as its node and as each saved node.
- **The tail:** it marks the start alone. That is an emit of the start (a pack's too), or the start loaded from a
  cursor and captured into a label. The tail may test the candidate and the loaded node.

At a start s, that scan asks whether a sibling after s passes the tail's tests. The question is the same at each
sibling, so:
- **The leader** is the walk at the parent's first child (the parent is in the move's frame; the first child is the
  parent + 1). It answers for every sibling in one pass over the children, by windows of slots: it finds the last
  child that passes, then marks each sibling before it that is a start of the pass (`root_admits`, one node of the
  pass's root rule) and passes the tests of the loaded node.
- **Every other sibling's walk** leaves the scan to the leader when the leader is a start of the pass too, and
  otherwise scans as before.

Nothing is precomputed: the leader reads the tape in place, and its marks are the label's own. The spill test
checks that 9 of its fixtures' walks have the shape, and the device's roots there are the CPU's.

At s1000 the shared shape is the `save0 after kind=238 load0 capture emit0` branch of a pack that every node starts
(load 19–22; the findings check here was the constant md5 above):

| | `label_micro_8` | `label_micro` | The pack's steps | Longest walk |
|---|---|---|---|---|
| Each start scans its siblings | 41.5–56.3 ms | 25.4–42.5 ms | 10,121,384 | 290 |
| The leader answers for all | 39.4–50.5 ms | 21.1–38.7 ms | 9,574,048 | 288 |

The pack's steps fell by 5.4%, within the timing noise. Its walks average 10.9 steps, and the `after` scans mostly
stop early, so at s1000 the n² hops were a small part of the work. The pass removes their growth with a block's
length. The longest walk is now a leader's: two passes over a long block's children, a window a hop where the
children are large.

## Open

- The longest walk at s1000 is a leader's 288 steps, against Q56's bound of 100. Its two passes are serial: the
  marks need the last passing child, which is known only at the chain's end.
