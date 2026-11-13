# Matrix-offload spikes: results

## Intro

The question: how much of the logic engine's relational work could run profitably on matrix hardware, if it were
restated as dense, brute-force work? The candidates on this machine (Apple M5 Max) are:

- the Apple Neural Engine (ANE), reached through Core ML;
- the CPU's matrix unit (SME2), reached by C with ACLE intrinsics;
- the GPU, both its plain compute threads and its matrix units (Metal 4 tensors, `matmul2d`).

Memory bandwidth was treated as the limiting resource, so the results count bytes as well as time.

The work came in two rounds:

1. **Microprobes** of each engine: dispatch cost, bandwidth, peak rates, exactness, and which operations run where.
   Details are in `ane/RESULTS-ane.md`, `cpu/RESULTS-cpu.md` and `gpu/RESULTS-gpu.md`.
2. **Five real workloads**, extracted from the data-layer checkout's 13,088 Python files (`extract/`). Each was run on
   every engine and checked against exact expected values. The CPU baselines are in the forms the engine uses today
   (hash joins, postings, semi-naive closure), next to their dense restatements (`data-notes/baseline-w*.txt`).

How times are reported:

- CPU work: CPU time (user + sys).
- GPU work: device time.
- ANE work: wall time (the minimum of many calls), plus the host's CPU time.

The machine's load average varied from 7 to 165 and is recorded with every table in the per-engine files.

## The corpus, as the workloads see it

| Quantity | Value |
|---|---|
| Documents | 13,088 |
| Named nodes | 22.3 M |
| Node kinds | 115 |
| Distinct names | 260 K |
| Identifier occurrences | 5.8 M (1.53 M bindings) |
| (document, name) pairs | 1.14 M |
| Import edges between documents | 105,691 |
| Strongly connected components | 10,824 (largest 2,198) |
| Document-to-document reach pairs | 29.7 M |
| Distinct names per document | p50 52, p99 602, max 2,764 |

Two facts shape everything below:

- **Every per-document domain is small.** 99.96% of documents have at most 2,048 distinct names, so local name ids
  fit in 16 bits and are exact in fp16. Kind codes fit in 7 bits.
- **The inputs are small; the outputs are not.** Each workload's inputs are 1–45 MB. The join's 83 M result rows take
  665 MB as two u32 columns, and the closure's 29.7 M rows take 238 MB.

## The five workloads

| Workload | What it is |
|---|---|
| W1 prefilter | For each (document, rule) pair, how many of the rule's names occur in the document. 13,088 documents × 444 rules over 767 rule names. |
| W2 join | Within each document, pair each read of a name with each binding of the same name. This is 0095's `tile` shape (`up_once`). |
| W3 closure | Transitive closure of the import graph between documents. |
| W4 group-by | A histogram of node kinds per document. |
| W5 co-occurrence | For every pair of the 2,048 most frequent names, the number of documents that hold both: MLDB's AᵀA overlap counts. |

## Results

Times are the best measured for each form. Ratios between columns compare different clocks (CPU time against device
time), so read them as orders of magnitude.

| Workload | Engine's form today (CPU) | Best CPU form | SME | GPU (device) | ANE |
|---|---|---|---|---|---|
| W1 prefilter | postings: 6.9 ms | — | **0.69 ms** | **0.077 ms** | 0.71 ms (rules as a 1-bit constant); 0.25 ms with documents as the constant, plus ~0.7 s to compile each document set |
| W2 join, counts | hash: 73 ms | direct table: **2.3 ms** | 497 ms (brute force) | **0.082 ms** (direct table in threadgroup memory); 2.15 ms brute force | ≥ 148 ms (estimated) |
| W2 join, emitting the 83 M rows | hash: 251 ms | — | — | 6.4 ms, limited by writes | — |
| W3 closure | semi-naive with a hash set: ~5.4 s | bit-row DP over the component DAG (NEON): **0.4 ms** | 236 ms (squaring) | 1.05 ms level by level; 12.1 ms squaring | ~2 s (estimated) |
| W4 group-by | hash on (document, kind): 234 ms | 8 sub-histograms: **5.5 ms** | — | **0.23 ms** (threadgroup atomics) | ~87 ms (scaled from a reduced run) |
| W5 co-occurrence | sorted-list merge: 2.27 s | bitmaps: 77 ms (NEON 57 ms) | **1.8 ms** | **0.38 ms** | 4.0 ms, plus summing 59 MB of partials on the host |
| All five in one GPU command buffer | 161 ms (sum of the best CPU forms) | | | **1.92 ms**, 0.15–0.18 ms host CPU | |

## Findings

### 1. The shape matters more than the hardware

On the CPU alone, choosing the right form gains 18× to 1,700× over the engine's forms today:

- the histogram: a direct histogram instead of a hash group-by;
- the join: a direct-indexed counting array over per-document local ids instead of a hash join;
- the closure: a bit-row DP over the condensed DAG instead of a semi-naive fixpoint;
- co-occurrence: bitmaps instead of sorted-list merges.

These gains come from the small coded domains above, not from any accelerator.

Brute force pays only where it is also the right algorithm:

- **Equality joins want counting arrays.** Over the same keys, all-pairs tiles are 26–200× slower, on every engine.
- **DAG closures want the DP.** Squaring is 100–600× slower.
- **Dense bit products** (W1, W5) are where brute force is the right algorithm.

### 2. The GPU is the main accelerator

The GPU wins every workload once each uses its best form. All five finish in 1.92 ms together.

- **Bandwidth:** about 550 GB/s from DRAM. Working sets under 32 MB read at 870–950 GB/s from the system-level cache.
  Every workload here fits.
- **Boolean work belongs on the ordinary threads.** Bit-packed AND+popcount reaches 36–76 Tbit-op/s. That beats the
  int8 matrix units (106–110 TOPS in bursts, about 43 sustained) and reads 8× fewer bytes.
- **The matrix units pay only above about 80–200 int8 operations per byte of DRAM traffic.** Group-by does not reach
  that: the one-hot form on the matrix units is 3× slower than threadgroup atomics.
- **At this corpus size the cost is in the framing, not the kernels.** One host round trip (0.13 ms at best, about
  1 ms when the GPU is cold) costs as much as a whole workload. 0095's device waves are the right shape: one command
  buffer per wave, at least about 1 ms of device work in each.

### 3. SME pays for dense bit products inside a CPU stratum

- **Peak:** BMOPA (a 1-bit outer product, XNOR+popcount) runs 23–26 Tbit-op/s on the P cluster, 40–50× NEON when the
  shared dimension is at least 16 K bits.
- **Real gains:** W5 runs 43× faster than the bitmap form, and W1 10× faster than postings.
- **There is one SME unit per 6-core cluster.** Threads on the same cluster share it, so the plan is one SME worker
  per cluster.
- **Streaming blocks must be long:** at least about 1,000 outer products to reach 90% of peak.
- **No dispatch cost**, which suits the engine's many small strata (5,000 strata under 5 ms each in the Python pack).
- **Elsewhere it does not help:** equality, histograms and sparse closures.

### 4. The ANE: the bandwidth idea checked against the measurements

What holds:

- **Compressed constants reach the chip compressed.** 1-bit palettized weights are 1/16 of fp16 in the compiled
  model, and are expanded on chip.
- **In fused chains, 1-bit constants run 3–6× faster than fp16** (21–41 TFLOPS against 6–13).
- **It is uncontended by the CPU.** Its minimum times moved under 10% at load 160, while it costs only 80–130 µs of
  host CPU per call.

What limits it, in practice:

- **Data that changes per run must be fp16 inputs, which can't be compressed.** In W1 and W5 the per-run side is the
  large one, so the ANE reads 15–16× the bytes the packed-bit forms read, and 24–35× counting its fp16 outputs. The
  compression only applies to data that is fixed across many runs.
- **Weight-bound passes don't speed up in proportion to the bytes saved.** 1-bit is only 1.9× faster than fp16 (an
  on-chip ingest rate of about 60–120 G entries/s).
- **Each call costs about 100 µs.** Core ML keeps work under about 0.5 GFLOP on the CPU.
- **Exactness:** counts are exact only up to 2048, outputs become inf above 32768, and K is limited to 16384.
- **Missing operations:** gather, sort and `select` all fall back to the CPU.
- **It competes with the GPU for bandwidth:** a busy GPU slows the ANE 1.1–2.6×.

Where it fits:

- **Near parity is a win.** The ANE runs W1 at SME's speed (0.71 ms against 0.69 ms) and W5 within 2.2× of it. Work
  it takes moves to a lower-power unit (reported 2–14× better energy than the GPU) and leaves the CPU and GPU free for
  other work. So it is a third concurrent lane, chosen when its price is within a band of the best lane's, not only
  when it is the fastest (decision 0098 §2).
- **Fixed relations become 1-bit constants**, against a batched per-run input. Candidates are a pack's rule
  vocabularies, library facts held once (0089), and the documents of a long-lived session. W1 with the documents as
  the constant took 0.25 ms (3.2× the GPU). Each constant costs about 0.7 s to compile, and the limit is about 119
  compiled models per process.
- **On raw speed and bytes alone, it loses to the GPU** on every workload here.

### 5. Where the bandwidth goes

- **No workload is DRAM-bound at today's sizes.** The best CPU kernels read 1–37 GB/s of unique input, against
  60–100 GB/s available to one thread or cluster.
- **Writing out rows is the expensive part.** W2's 83 M rows cost 251 ms of CPU and 6.4 ms of GPU time (at 109 GB/s,
  limited by writes). W3's closure is 238 MB as rows.

The bandwidth levers are therefore:

- **Don't materialize what a count, bitset or per-document local id can answer.**
- **Store the narrowest encoding:** u16 local ids or 7-bit kinds instead of i64 columns, and sorted runs or bitmaps
  chosen by density (they break even around 1/16).
- **Keep a wave's working set under about 32 MB** so the GPU reads from cache.

Compressed operands help only where they can stay compressed:
- on the GPU and SME, packed bits;
- on the ANE, constants only.

## What this means for the engine

Mapped onto draft 0095's plan ("Relational work on Metal": device waves, physical forms chosen by the planner):

| Physical form | Engine | Covers |
|---|---|---|
| `tile`, per-document counting table | GPU threadgroup memory; NEON on the CPU | W2: 0.082 ms GPU, 2.3 ms CPU |
| Bit-product form | SME on the CPU (one worker per cluster); popcount on GPU threads | W1, W5, set overlap, semi-join batches |
| DAG closure | SCC condensation + bit-row DP: CPU NEON, or GPU level by level | W3 |
| Direct group-by | GPU atomics; CPU sub-histograms | W4 |
| Bit products against fixed relations, as compressed constants | ANE, as a concurrent low-power lane | W1 at SME's speed, W5 within 2.2× |

- **The planner's choice of form gains more than the choice of hardware.**
- **Dense matmul is rarely the right relational form.** The matrix units (ANE, GPU tensors) pay only where the work is
  a real dense product with many operations per byte.

## Other platforms

These are from the research brief (sources in the session notes), not measured here:

- **Bit products port widely:** NVIDIA's 1-bit AND+popcount MMA, Arm SME2 elsewhere, Intel AMX (int8).
- **Counting tables and atomics port to any GPU.**
- **AMD XDNA2's open toolchain** (IRON / MLIR-AIE, custom C++ per tile, full integer and bitwise operations) is the one
  NPU that could run these forms directly, rather than as fp16 matmuls.
- **NPUs that run only fixed graphs** (Intel NPU, Edge TPU) have the ANE's limits or worse.
- A CIDR 2026 study of on-chip NPUs in a database found them well behind a GPU for filter, join and average.

## Limits and open items

- **One corpus, one language.** The workloads are extracts with the engine's shapes, not the engine's own relations.
  0095's step 0, a relational profile of the data layer, would say how much of a real run each workload stands for.
- **GPU clocks are noisy under load.** Sustained rates are about 2.5× lower than bursts.
- **The ANE was measured through Core ML only.** A private route (`_ANEClient`) might lower the per-call cost, but
  would not change the fp16-input rule.
- **Not built:**
  - a cooperative GPU write for W2's rows, estimated at about 1.4 ms;
  - SME bitmap containers in an interleaved layout;
  - a sweep of the GPU tensor unit's int4 and bf16 types.
- **Product paths are not settled:**
  - SME from Rust needs inline assembly, since Cranelift has no SME;
  - the ANE needs Core ML models built offline (coremltools is a dev-only Python tool here).
