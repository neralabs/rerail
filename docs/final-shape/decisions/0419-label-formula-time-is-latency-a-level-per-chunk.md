# 0419. `label_formula`'s time is latency, a level per chunk: a chain's reach by blocks, chunks grow until measured

Status: accepted (2026-11-18). Follows 0425 and Q218.

## Intro

0425 removed about a quarter of `label_formula`'s (file, formula) pairs and half its formula images, and the kernel's
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

- **The pairs 0425 removed were cheap.** A seed was a one-step image. A dispatch of plain formulas (leaves, AND, OR)
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

- **Chunking is not the lever with the memory line lifted** (followed up the same day). The probe-size gain above
  was under the default line. With the line lifted, s1000's label GPU time did not follow the chunks:
  - with a larger cap (`--chunk` 524,288 or 1,048,576), 67–103 ms against 69–84 ms;
  - with one chunk at a time and a 1,048,576-node cap (`RERAIL_CHUNKS_AT_ONCE=1`), 90–107 against 107–130 ms
    (internal_dead_code) and 84–99 against 70–106 ms (public_api_triples), at load 27–57.

  These are within run-to-run noise. The rows were the same. The profile's chunk count moved only between 6 and 8.
- **The per-node measure is noise, not a model.** A credible chunk added 16–113 MB whatever its nodes (66,499 nodes:
  89 MB; 70,985: 113 MB; 78,447: 32 MB; 259,488: 93 MB). No bytes-a-node rule fits that, so none was adopted. A
  rule that sized chunks by the chunks of at least a quarter of the largest measured was built, and dropped
  unmeasured as a gain. Its patch is kept outside the tree.
- **Gathering concurrent chunks' label runs is not built.** Its prize is fewer label runs, and fewer runs (one chunk
  at a time, larger chunks) did not lower the GPU time measurably. Decision 0386 keeps each job's wave its own, so
  gathering would also make chunks wait on each other.
- 0425's GPU numbers were taken under the default memory line (`RUN_MB` 1024), which cut its chunks smaller. Its
  before-and-after comparison stands, since both sides ran under the same line. Its absolute times are those of
  14–17 chunks.

## A formula's words over the chunk: tried, not kept

Asked "why are we doing many dispatches of small, latency-bound work?" (the user, 2026-11-18), I proposed evaluating
a formula's sets, ANDs and ORs over the chunk's words: a thread per word in a new kernel, `label_formula_words`, with
the formula's stack in registers. Only the steps would stay in the per-(file, formula) groups, run one step per
dispatch. It was built, with the rows the same (17c18ee8), and was not kept. The patch is kept outside the tree.

- **No gain.** Label GPU time, alternating, at load 25–42, before against after:
  - internal_dead_code: 80–96 ms against 79–99 ms;
  - public_api_triples: 79–84 ms against 76–94 ms.
- **The steps hold the time.** Per kernel (`RERAIL_GPU_KERNEL_TIMES`): the word passes took 14 ms in all, over 217–244
  dispatches. The step runs alone took 64–70 ms, as much as whole formulas had (65–79 ms). The sets, ANDs and ORs
  were never the cost.
- **It broke the launch budget.** The pack's wave went past `KERNEL_LAUNCH_LIMIT` (200) per submission:
  `the_wave_is_one_evaluation_on_metal` and `the_wave_finds_ws_matches_on_metal` failed with `BackendLaunchLimit`. A
  shape that adds dispatches per step does not fit the budget.

The steps' time by op (one formula a dispatch, so fixed costs are inflated): a chain's reach (`REACH_CHAIN`) 68.7 ms
over 60 passes, 1.1 ms each; the children of members (`PARENT`) 36.6 ms over 540 passes, 68 µs each; descendants
(`REACH_DESCENDANT`) 10.7 ms over 340; a child within a conjunction 7.1 ms over 312. A chain's reach is the long
pole: Q220.

