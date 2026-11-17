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
merge keeps oscar's contents in 0289's form. That is the hub's roll-up to make, so it was not made here.

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
