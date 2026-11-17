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
3. **Matcher walks node-parallel, not root-serial.** A `desc kind=K` is a range query on the preorder tape (the
   nodes of kind K between a root and its end): a per-kind position list and two binary searches, or one thread per
   node testing its ancestors' range. A `child` loop is a segmented pass over the children. Long walks then cost
   their output, not their length.
4. **Overlap the host's part with the device's.** Encode the next launch while the GPU runs this one, and move the
   host-side text scans and sizing onto the device.
5. **Strata over many chunks in one submission**, and a recursion's rounds in one submission. A stratum's runs
   across chunks are independent until the cross part.
6. **The executor's matcher prices from measured samples**, now that the CPU it measures is right.
7. **Device forms for the computed-term hazards** (`@concat`, `@rr_node_text`, `@pkg_up`, …): 32 strata, 4.1 s.
