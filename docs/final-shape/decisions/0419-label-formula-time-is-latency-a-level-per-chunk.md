# 0419. `label_formula`'s time is latency, a level per chunk: a chain's reach by blocks, chunks grow until measured

Status: accepted (2026-11-18). Follows 0406 and Q218.

## Intro

0406 removed about a quarter of `label_formula`'s (file, formula) pairs and half its formula images, and the kernel's
time did not fall. The user asked: "1. identify why the GPU time didn't drop 2. do Q218: evaluating each level's
formulas together per file." Asked which way to go (evaluating per file together looked slower than what it replaced),
they said: "I want both, and I also want you to survey the code for existing lauch-delay and chunking affordances and
reuse theinfrastecture and patterns already in the tree".

`label_formula` runs one threadgroup of 128 threads per (file, formula), over a dispatch's formulas
(`formula_group`). A formula is postfix set operations over the file's words: leaves (`label_leaves`), ANDs and ORs,
and steps (children, descendants, a chain's reach).

## Why the GPU time did not drop

A trace switch (`RERAIL_FORMULA_TRACE=<file>` with `RERAIL_GPU_KERNEL_TIMES`, one line per dispatch: its formulas, their
opcodes, the files, its µs; `RERAIL_FORMULA_ONE` dispatches one formula at a time) showed this on internal_dead_code,
s1000, at load 18:

- **The pairs 0406 removed were cheap.** A seed was a one-step image. A dispatch of plain formulas (leaves, AND, OR)
  takes about 140 µs whatever it holds: 224 such dispatches took 40 ms. The time goes to the step formulas: 872
  dispatches, 245 ms (each committed and waited for apart in trace mode, so the sums are larger than a run's).
- **One formula shape was the long pole:** a chain's reach (`REACH_CHAIN`, e.g. `13.6.4.5.4.25`). Each member's
  subtree was walked on one thread, a device read a node, so a dispatch waited for its largest member.
- **The work is small; the cost is latency.** Under `RERAIL_PROF` the whole run is 2.76 M child reads, 14,382 chain
  members and 22,520 (file, formula) groups. A level is a few dependent dispatches, and every level runs again per
  chunk: the corpus is evaluated chunk by chunk (0029, 0254), and each chunk's `demand` makes its own Runner.
- **The chunks were small.** Until a chunk adds 16 MB (`CHUNK_CREDIBLE_MB`), it sizes nothing, and the next was
  `PROBE_NODES` (65,536) again. At s1000 that was 14 chunks and 26 label runs. Started at 262,144 nodes
  (`RERAIL_PROBE_NODES`, a measurement switch), it was 4 chunks and 8 runs. The label passes' GPU time went from
  96–101 ms to 42–56 ms, CPU from 7.9–8.4 s to 7.3–7.4 s, and instructions from 57.5–58.8 G to 55.1–55.3 G. Rows were
  the same (load about 100, under the default memory line).

## Decision

1. **A chain's reach by blocks** (`REACH_CHAIN` in `label_formula`). This is the CPU's `Reader` rule: a node is
   reached where its parent is a member, or its parent is in `through` and reached. Each thread takes a block of
   words, in preorder, as a descendant image does. A parent inside the block is already decided. A parent before the
   block climbs while its ancestors are in `through` and not members. The chain formula went from 96.8 ms to 54.6 ms in
   one-formula mode. The best of three runs per formula shape went from 66.9–74.6 ms to 49.2–66.2 ms. A node-parallel
   climb over every node was tried first: 244 ms in total. It was dropped.
2. **Chunks grow until one measures** (`UNCREDIBLE_GROWTH = 4` in `run_observed`). A chunk below the credible growth
   makes the next one four times its size, at most the caller's `--chunk`. One chunk still runs at a time, so a larger
   one measures again. This reuses the existing sizing (`PROBE_NODES`, `CHUNK_CREDIBLE_MB`, the chunk-size decision
   in the profile) and does not add a mechanism. Measured with the memory line lifted (`RERAIL_RUN_MB=20000`), pinned,
   at load 27–43, alternating pairs. Label GPU time:
   - internal_dead_code: 82.5–98.2 ms → 77.0–84.2 ms.
   - public_api_triples: 89.5–93.8 ms → 67.1–82.4 ms.

   Natively both are the same: internal_dead_code 5.6–5.8 s of CPU before and 5.6–5.7 s after, public_api_triples
   6.3–6.6 s and 6.4–6.6 s. Rows are the same.
3. **Evaluating a file's formulas together is a measurement switch, not the default** (`RERAIL_FORMULA_FUSED=N`).
   `label_formula`'s groups take formulas `group.y`, `group.y + grid.y`, …, in the same scratch slots. With the
   switch, a dispatch holds all of a level's formulas and N groups per file. At 1, one group evaluates every formula of
   its file in turn. Runs with the memory line lifted, pinned, alternating, at load 54–70. Rows were the same in every
   run.

   | Program | Label GPU, default | `FUSED=4` | `FUSED=1` |
   |---|---|---|---|
   | internal_dead_code | 83.0–94.9 ms | 101.9–121.2 ms | 164.7–174.8 ms |
   | public_api_triples | 70.9–79.0 ms | 86.9–92.7 ms | 105.1–112.2 ms |

   The groups of one file's formulas overlap their latency on the GPU, and serializing them removes that overlap. The
   tests shared within a level are already shared: the leaves are evaluated once per level (`label_leaves`), and
   identical formulas once (`formula_group`'s `unique`).
4. **`RERAIL_FORMULA_WIDTH`** (32–1024 threads per group; `partials` is now 1024 words) is a measurement switch. At
   load 120 the width A/B was within noise.

## What is left

- **The chunk count is now set by the memory sizing.** With the line lifted, s1000 runs in 6–8 chunks. Once a chunk is
  credible, the next is the chunk budget (384 MB) over the most bytes per node any chunk added this run: 1,462 B, from a
  70,985-node chunk, gives about 275,000 nodes. A larger `--chunk` (524,288 or 1,048,576) left the count at 6–8, and
  the label GPU time within noise (67–103 ms against 69–84 ms). Small chunks' per-node figure includes fixed costs,
  so the largest is pessimistic. That is the memory budget's rule (Q74), and it is kept.
- **Gathering the label runs of chunks that run at once** into one Runner (the lanes wave's `queue`/`flush_group`
  pattern, 0113, 0123) would cut the per-chunk levels further. It is not built. Each chunk's label run reads its own
  chunk's tapes and residency, so this needs a Runner over several chunks' documents.
- 0406's GPU numbers were taken under the default memory line (`RUN_MB` 1024), which cut its chunks smaller. Its
  before-and-after comparison stands, since both sides ran under the same line. Its absolute times are those of
  14–17 chunks.
