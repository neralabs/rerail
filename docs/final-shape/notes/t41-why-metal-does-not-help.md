# t41: Why a Metal run costs what a CPU run costs

2026-11-16. internal_dead_code and public_api_triples over the data-layer corpus (9,010 Python files, 3,110 read),
strict, cold, `--metal`, on an M5 whose load ran from 15 to 68. Small runs first: every k-th file (s10, s100,
s1000), then the whole corpus.

## Intro

**The question.** The GPU is much wider than the CPU and has much more memory bandwidth, yet a `--metal` run takes
the CPU and wall time of a CPU run. At s1000 the two are equal within noise (about 4 s CPU each, equal
instructions). Over the whole corpus, internal_dead_code on Metal took 132 s CPU and 23.6 s wall.

**The answer, in short.** Four causes, each measured:

1. A measurement bug priced the GPU's matcher launches 300 times too high, so most of them ran on the CPU.
2. Most of the CPU is work with no device form. W's reference graph alone is 39% of it.
3. The device gets tiny grains. The run's memory line cuts chunks to the minimum, and every stratum and matcher wave
   runs once per chunk.
4. The matcher's device form is latency-bound: a GPU thread per root walks the tree serially, and a few long walks
   set each dispatch's time.

The device's speed was never the limit: it is idle, or waiting on one thread's chain of loads.

## 1. The GPU's matcher launches were priced at 158 ms each (fixed)

A GPU job begins on its lane's thread, where the scoreboard read the thread's CPU clock. It ends on the lanes' host
pool (`lane_host`), where the scoreboard read *that* thread's clock. The difference of two unrelated clocks gave
158 ms a job. The lane's total (169 s) was past the whole process's. `measured_prices` takes `job_cpu` as the GPU's
per-submission host cost, so every GPU matcher launch was priced at 158 ms of host CPU.

| internal_dead_code, Metal | matcher batches on the CPU | on the GPU |
|---|---|---|
| before | 282 (12.1 s wall) | 194 (5.6 s wall) |
| after the fix | 39 | 433 |

Fixed: a ticket pauses when it leaves a thread and resumes on the next, keyed by thread, and carries its phases
(they were thread-local too, so a job's "cold" flag could come from another job). Test:
`a_job_handed_between_threads_counts_only_its_own_cpu`.

## 2. Most of the CPU has no device form

The profile's new summary (`== Metal: what ran where`), internal_dead_code on the whole corpus:

| own CPU | s | share |
|---|---|---|
| W's reference graph (a second tree-sitter parse, summarized in Rust) | 39.7 | 39% |
| matchers' host parts (captures, confirmations at each root) | about 10 | 10% |
| relational strata, all runs | 11.5 | 11% |
| the parse and tape | 5.5 | 5% |

- **W's reference graph** (`references::add_file`) parses each file again with tree-sitter and runs W's
  `calls::summarize`. That costs 12.6 ms a file, against about 1–2 ms for the parse. The sample's hot frames are
  `ts_node_child_with_descendant`: tree-sitter's `parent()` walks down from the root on every call, so climbing
  parents node by node is superlinear in depth. It had no span, so "tree-sitter and facts" hid it; now it has one.
- **Relational strata** are 11% of the CPU. Even all of them on the GPU at no cost would save 11%.

## 3. The device gets tiny grains

A chunk is the least of 384 MB and three quarters of the headroom under the run's memory line (`RUN_MB`, 1 GB). The
run's footprint is past 1 GB from early on (1.1–1.4 GB), so 455 of 498 chunk sizings fell to the minimum: 457 chunks
of 8–10k nodes. Every stratum runs once per chunk (29,887 stratum runs), and so does every matcher wave.

- **Strata.** A run averages 0.29 ms of CPU, below a GPU submission's cost, so placement rightly keeps 215 strata
  on the CPU ("priced cheaper").
- **Matchers.** Each launch pays its dispatch's tail (below). Raising the line (`RERAIL_RUN_MB=8000`) gave 34 chunks:

| internal_dead_code, Metal | 457 chunks | 34 chunks |
|---|---|---|
| matcher batches | 472 | 70 |
| matcher kernels, GPU time | 5.5 s | 1.28 s |
| peak footprint | 2,501 MB | 2,537 MB |
| CPU (load 43, then 16) | 122.8 s | 111.1 s |

The peak did not move: whatever holds the footprint past the line is not the chunks. So the memory line, as it
stands, costs the device its grain and buys nothing.

## 4. The matcher's device form is latency-bound

`RERAIL_GPU_KERNEL_TIMES=1 RERAIL_GPU_WALK_STEPS=1` at s1000:

| kernel | GPU ms | dispatches | threads |
|---|---|---|---|
| `label_micro` (the loop tier, t2) | 119.4 | 12 | 27,144 |
| `label_micro_8` (t0) | 72.1 | 35 | 902,056 |
| `clause_patterns` | 3.0 | 19 | 17,775,373 |
| `label_settle` | 0.5 | 38 | 2,330,580 |

- `clause_patterns` runs 17.8 M threads in 3 ms: the GPU at its best.
- `label_micro` runs about 2,260 threads a dispatch, far too few to fill the GPU, and takes 10 ms a dispatch. Its
  walks are a thread per root: `child … jump` loops over siblings and `desc` scans of a subtree, averaging 420–800
  steps and reaching 2,345–4,280. A SIMD group lasts as long as its longest walk. Each step is a dependent load, so
  the dispatch is one thread's critical path, and decision 0125's spill (past 512 steps) only shortens the tail.

The GPU lane is one slot, and within a launch the host's part (sizing, tapes, encoding, the text scans: 1.76 s over
32 launches) and the GPU's part run one after the other.

## Also found

- **The executor's matcher estimates are 3× the actual on the GPU** (16.3 s estimated, 5.0 s run, 34 chunks), so
  38 of 70 batches still went to the CPU.
- **Profile gaps, fixed:**
  - the acceleration section's "lane CPU" was always 0 (`Report::lane_cpu` was never filled);
  - the GPU strata table summed a stratum's CPU runs into its GPU CPU (xd_raw_parameter_annotation: 85 of 87 runs on
    the CPU);
  - `RERAIL_GPU_KERNEL_TIMES` left the Device table's GPU column at 0;
  - nothing flagged chunks the memory line cut.

## What the profile now shows

- `== Metal: what ran where` near the top of the profile:
  - spanned CPU;
  - strata runs on the GPU, then on the CPU by why (no device form, refused, priced cheaper), with CPU ms a run;
  - launches by lane, with wall and job CPU;
  - the label passes' GPU time, and the walks' steps or how to get them;
  - the CPU with no device form;
  - the chunk sizings the memory line cut.
- The GPU strata table splits each stratum's GPU runs from its CPU runs (CPU, runs, work).
- A warning for chunks cut by the memory line.
- A span for W's reference graph, with a kind of work of its own.

## What it takes to make Metal pay

In order of what each buys:

1. **The reference graph off its second parse.** 39% of the CPU is W summarizing a second tree-sitter tree with
   `parent()` climbs. Rebuild it from the tape and facts already made (a rule program, or the transducer), or at
   least replace the parent climbs with a cursor. This is CPU work either way, and the largest single item.
2. **Chunks sized by the work, not starved by a line the run is already past.** The peak did not move with 13×
   bigger chunks. Find what holds the footprint past 1 GB, then let chunk size follow the device's grain. On the
   GPU path the right grain is the whole corpus or a few large slabs, as the matcher's 4× drop in GPU time shows.
3. **Matcher walks parallel over roots and over a long walk's range, on the tape as it lies.** Not by posting a
   kind's nodes (0293 §6: nothing is posted), but by a scan of the codes: a `desc kind=K` is the codes of the
   root's subtree range (preorder, contiguous) tested 64 at a time, the groups spread over threads; a `child` loop
   is a segmented pass over the children. Long walks then cost their output, not their length. (Corrected
   2026-11-16: a first draft here proposed per-kind position lists.)
4. **Overlap the host's part with the device's.** Encode the next launch while the GPU runs this one, and move the
   host-side text scans and sizing onto the device.
5. **Strata over many chunks in one submission**, and a recursion's rounds in one submission. A stratum's runs
   across chunks are independent until the cross part.
6. **The executor's matcher prices from measured samples**, now that the CPU it measures is right.
7. **Device forms for the computed-term hazards** (`@concat`, `@rr_node_text`, `@pkg_up`, …): 32 strata, 4.1 s.

## After the hub merge, and against oscar's tape on the device (2026-11-16, later)

The sibling branches are rebuilding the engine's structures to be the same bytes on the CPU and the GPU
(0260, 0266, 0289, 0295, `data-structures.md`) and the tape to be the one form both read, in place
(0293, 0310: the reads written once in CLIF for Rust, the CPU JIT and MSL). This branch has merged the hub
(61e9f383c), which holds 0293's CPU half and 0295 step 5. Oscar (`sync-integration-hub-tape`, f51cab698) holds the
device half (0293 §9, §10) and 0310, not yet in the hub. Its merge here conflicts in 17 files, in one place: the
document view, `ChunkTape` (0289 §4: a block's offsets from the document's start, from `structure!`) against
`rerail_clif::view::TapeView` (0310: the one view the generated reads take, with nothing per node derived). The
merge keeps oscar's contents in 0289's form: one view (`TapeView`, `at` its first word), its parts offsets from
`at`, one block bound. Merged here (2026-11-16, below).

**On the hub's shape, nothing above changes** (internal_dead_code, whole corpus, at load 25–31: the reference
graph 35.2 s, 393 of 443 chunk sizings cut by the memory line, 858 M walk steps, the longest 6,607).

**On oscar's, the device's matcher is 3–4× slower.** Measured on a worktree of f51cab698 with its own target,
at s1000, alternated with the hub's build at load 24–35 (`RERAIL_GPU_KERNEL_TIMES`, `RERAIL_GPU_WALK_STEPS`):

| s1000, Metal | hub (61e9f383c) | oscar (f51cab698) |
|---|---|---|
| `clause_patterns` threads | 18,080,805 | 186,381 |
| `label_micro_8` threads | 689,796 | 220,037 |
| `label_micro` threads | 23,255 | 17,212 |
| micro kernels' GPU time | 139 ms | 474 ms |
| their walk steps | 68.7 M | 43.9 M |
| GPU per step | about 2 ns | about 11 ns |

- **The cause is the execution shape, not the shared bytes.** 0293 §10 makes a GPU thread "a pass and a group of
  64 of a document's nodes", and "a group's roots run one after another in its thread". That is the CPU's shape,
  a core looping over a word of matches. On the GPU it serializes up to 64 walks a thread: `clause_patterns`, the
  run's best kernel, went from 18 M threads to 186 k.
- **A parent is a search on the device now** (0293 §9: format 15 searches back 64 nodes, then descends from the
  root). The micro walks step `parent` often. On the CPU, 0293 §6 kept each projection's parents on measurement
  (a pass off the tape costs 45 to 100 times a pass over them); the device dropped its parent column with no such
  measurement. Format 16 (`RERAIL_TAPE=16`) was no better per step on oscar's device (about 12 ns).
- **Keep the shape, change the device's form of it.** The host still lists nothing. A group's 64-bit root mask is
  made by its thread, and then its roots are a SIMD group's: each lane takes a set bit (a ballot over the mask, or
  a scan of the masks' counts that gives each root a thread). A walk's frame carries its parent where its step knew
  it (0293 §8 does this for the CPU's `TapeNode`), so a parent search is the exception.
- **Found too:** oscar's engine stops on the whole corpus, in both runs: `setfix.rs:3043`, index 5 of a view row's
  2 columns, in `step_histograms` (exact statistics, 0203), not on the device. The hub's tree runs the corpus. Oscar
  lacks the hub's later `setfix.rs` changes, so it may be gone at oscar's next hub merge.

## Oscar merged here

Oscar's f51cab698 is merged into this branch. Each side's invariants are kept:

- 0289 §4: one block per batch, and the view's parts as offsets from `at`.
- 0293 §9–10: nothing per node derived, and roots as a rule.
- 0310: one view definition, and the reads through CLIF.

At s100 with `--metal`, warm, the findings are the hub's (none). The label passes' GPU time is 53.6 ms on the hub's
tree and 239.7 ms on the merged tree, the Q53 shape. The CPU is 1.6 s against 1.9 s, and the instructions 22.8 G
against 27.6 G.

The first run of a new engine compiles its shaders (22.8 s of CPU here), so the first run after a kernel change is
never a measurement. Q53 is now this branch's to fix.

## Fixing the merged device (2026-11-16, later)

internal_dead_code at s1000, every matcher launch pinned to the GPU (`RERAIL_LANE_PIN=gpu:ast matcher wave`; without
it a run's placement differs from the next's), `RERAIL_GPU_KERNEL_TIMES=1`, `RERAIL_GPU_WALK_STEPS=1`, each engine
warmed by an s10 run first (its first run compiles its shaders), alternated, at load 45–75. The label passes' GPU
time, then three of its kernels:

| Engine | Format | Label passes | `label_micro` | `label_micro_8` | `clause_patterns` |
|---|---|---|---|---|---|
| hub (61e9f383c) | 15 | 292 ms | 169 | 91 | 3.6 |
| merged (88f4b24b6) | 15 | 1,235 ms | | | |
| + a group's roots spread over its SIMD group | 15 | 1,110 ms | | 497 | |
| + reads in 32 bits where only the low word is used | 15 | about 1,080 ms | about 490 | | |
| + block-local values declared in their block | 15 | 887–902 ms | 388 | 471 | 10.5 |
| + the batch's format a function constant | 15 | 781–801 ms | 322–336 | 429–434 | 6.0–7.0 |
| the same engines | 16 | 895 → 763–781 → 671 ms | 506 → 392–410 → 364 | 338 → 323 → 272 | 15.9 → 18.5 → 6.5 |
| hand-written format 15 reads (a diagnostic build) | 15 | 667 ms | 211 | 434 | 6.0 |

What each step is:

- **Roots over a SIMD group** (`simd_roots`, `metal_tape.metal`). A thread still makes its group's 64-bit root mask
  (0293 §10: the host lists nothing). Then the SIMD group's 32 lanes take its roots in rounds of 32, a lane per set
  bit, found by a prefix sum over the lanes' counts. `label_micro*` and `clause_patterns` run whole SIMD groups.
- **A walk carries its parent** (`path.end`): a step into a child knows the parent, so `next_in` needs no parent search.
- **32-bit reads** (`rerail_clif::msl`, `low_words`). A demanded-bits pass emits an I64 value as `uint` where every use
  reads its low word, as the documents' offsets do.
- **Block-local declarations.** A value that only its own block sets and reads is declared there, not at the
  function's head.
- **The format as a constant.** The label and pattern kernels are made per batch format, as function constant 0,
  `tape_format`; a mixed batch gets 0, which reads the view's field. The reads' format test then folds away.
  `DocTable::format` gives the format. The profile has `gpu.tape_format=N` and each label kernel's
  `pipeline.K.max_threads` (every one is 1,024, so registers don't limit occupancy).

What is left, by kernel:

- **`label_micro`: the generated reads are still 1.5× the hand-written ones** (322 against 211 ms). They still
  load the widths from the view and branch on them, and decode a column of any width. The widths are a document's,
  so a constant per batch does not reach them. Q57.
- **`label_micro_8` is parent climbs**, and the hand-written reads did not change it (434). Format 15's parent is a
  search; format 16's is cheaper, at 272. The hub's tree had a parent column (91). Making the device's parent cheap
  is format 16 as the default (0156, the user's), or a parent the walk carries further, or a column, against 0293 §9.
- **The step count is bimodal**: 97,241,194, or 118,095,951 with `label_micro_deep` at 70 ms instead of 27. Either
  engine shows both; the second is walks retried on the deep tier. Compare runs with equal counts only.
- The device's label tests passed with MSL that did not compile, and with a walk that lost its parent at a cursor
  restore. The conformance tests and these runs caught both. `the_label_kernels_compile_for_every_format` (slow
  tier, 84 s on an M1 Max) now compiles every label kernel and the pattern kernel for formats 0, 15 and 16. A test
  for a walk whose steps change is still Q57's.

### The generated code as structured control flow (2026-11-17)

`rerail_clif::msl` emitted every function with more than one block as a dispatch loop: `switch (b)` inside a
`while`, each jump a store to `b`. Metal's compiler keeps the loop's state live across the switch and does not merge
the blocks, so every read cost a branch through the dispatch. The emitter now writes the CFG as structured code:

- an `if`/`else` that rejoins at the branch's immediate post-dominator;
- a natural loop as `while (true)` with `continue` and `break`, or a selector `e<h>` where a loop has several exits;
- a block reached on two paths duplicated, up to a size cap (`STRUCTURED_MAX`) and a depth of 64;
- the switch form kept only as the fallback, for a CFG that is not reducible. No kernel in the tree needs it;
  `metal_tape_reads.metal` has no `switch (b)` left.

Same run as above (s1000, the GPU pinned, 97,241,194 steps in both), format 16, load 33–41:

| Engine | Label passes | `label_micro` | `label_micro_8` |
|---|---|---|---|
| merged, the dispatch loop | 710 ms | 355 | 315 |
| + structured control flow | 349 ms | 159 | 171 |
| hand-written format 15 reads (the diagnostic build) | | 211 | |
| hub (61e9f383c), with its parent column | 292 ms | 169 | 91 |

The generated reads now beat the hand-written ones (159 against 211 ms). The width branches were not the cost.
What is left against the hub's tree is `label_micro_8`, the parent climbs (171 against 91). The format 16 parent is
still a search where the hub's tree had a column.

### The parent climb spike (2026-11-17)

Q57 put `label_micro_8`'s gap to the hub engine (171 ms against 91 ms) on format 16's parent climb. A spike tested
that before building a window parent pass. Every variant was switched at compile time, with the same run as above,
alternated at load 45–96:

- **An oracle parent column**, made by a kernel before the walks, which the walks read in place of the climb:
  220 ms. No gain.
- **The window's parents from one forward pass** over its 64 slots, read where the node is in the start's window:
  170 ms. No gain.
- **Counted**: 4.0 M parent climbs in 97 M steps, 1.8 hops each. 92% of them land inside the start's window, and
  0.09 M steps escape to the side list.
- **The root mask made twice**: +8–16 ms, so making it is about 10 ms.
- **A group's nodes split over S threads**, at the same walks:

| Threads a group | `label_micro_8` | Label passes |
|---|---|---|
| 1 (before) | 179–180 ms | 376–378 ms |
| 8 | 110–112 | 272–274 |
| 16 | 94–110 | 250–301 |
| 32 | 94–97 | 240–243 |
| the hub engine | 76–96 | 247–258 |

The cost was the dispatch's shape. A thread a 64-node group gave a launch about 8,800 threads, and a SIMD group walked
its 32 groups' roots (about 560) in rounds, one after another. Decision 0359 makes a short walk's group a SIMD
group's. The loop tier keeps a thread an item: its pool is fixed at 4,096, and 1, 4 and 32 lanes measured the same
there.

The spike's split had a bug: it did not scale the loop tier's dispatch, so it skipped some of that tier's walks.
Its `label_micro` times are not valid; its `label_micro_8` times are, because that tier's walks matched exactly. The
profile now prints each tier's walks and steps over all passes, so a run whose walks differ is seen at once.

