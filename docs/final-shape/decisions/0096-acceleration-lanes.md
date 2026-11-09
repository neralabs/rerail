# 0096: Accelerating relational work: forms first, then four lanes (CPU, SME, GPU, Neural Engine)

Status: draft (2026-11-09). A plan for review. It extends 0095 and builds nothing yet.

## Intro

**The problem.** Decision 0095 plans the logic engine's joins, negations, aggregates and recursion on the GPU. The
matrix-offload spikes (`tools/matrix-offload/RESULTS.md`) then measured five relational workloads on every compute
engine of an M5 Max:

- the CPU's cores (NEON);
- the CPU's matrix unit (SME2);
- the GPU's threads and its matrix units;
- the Neural Engine (ANE).

Three results change the plan:

1. **The form matters more than the hardware.** On one CPU core, the right physical form beats the engine's current
   forms by 18× to 1,700×. The right forms are a counting table per document, a direct histogram, a closure by
   dynamic programming over the condensed graph, and bitmaps. 0095's form list does not have them.
2. **The engine has more than one accelerator.**
   - SME is a matrix unit inside the CPU with no dispatch cost. On dense bit products it runs 10–43× faster than the
     CPU forms it replaces.
   - The GPU wins every workload once each workload has its best form.
   - The Neural Engine comes within 1–2.2× of SME on bit products. Near parity is a win for the ANE (the user,
     2026-11-09): the work moves to a lower-power unit and leaves the CPU and GPU free for other work.
3. **Bandwidth goes on rows written out, not on inputs.** A join's 83 M output rows cost 251 ms of CPU. When the
   reader only needs a count, a counting table answers the same question in 2.3 ms.

**What changes.**

- The plan gets four new physical forms, which every backend runs (§1).
- Placement chooses among four lanes, not between two backends, and it counts the lanes' load (§2).
- The Neural Engine gets a product path that needs no Python (§3).
- SME gets shared hand-written kernels (§4).
- The order of work puts the CPU forms first, because they pay on every machine and at every corpus size (§5).

**How to read this record.** First the measurements it rests on: the spikes, and what the engine's real strata look
like. Then the decision in five parts, the order of work, the gates, and the open questions.

## What it rests on

### The spikes

These are measured over 13,088 Python files of the data-layer checkout: 22.3 M nodes and 260 K names. Times are the
best per form. CPU times are user + sys; GPU times are device time; Neural Engine times are wall time.

| Workload | Engine's form today | Best CPU form | SME | GPU | ANE |
|---|---|---|---|---|---|
| Prefilter: how many of each rule's names occur in each document | postings 6.9 ms | — | 0.69 ms | 0.077 ms | 0.71 ms; 0.25 ms with the documents as the constant |
| Join within each document, counts only | hash 73 ms | counting table 2.3 ms | (brute force: 497 ms) | 0.082 ms | ≥ 148 ms (estimated) |
| The same join, writing its 83 M rows | hash 251 ms | — | — | 6.4 ms | — |
| Transitive closure of the imports | semi-naive ~5.4 s | bit-row DP over components 0.4 ms | 236 ms | 1.05 ms | — |
| Histogram of node kinds per document | hash 234 ms | 8 sub-histograms 5.5 ms | — | 0.23 ms | ~87 ms |
| Co-occurrence of the 2,048 most frequent names | sorted merge 2.27 s | bitmaps 77 ms | 1.8 ms | 0.38 ms | 4.0 ms |

These Neural Engine facts constrain §3:

- **Only constants are compressed.** A constant goes into the model as 1-bit palettized weights, and is expanded on the
  chip. Per-run inputs must be fp16, 16 bits per bit of data.
- **A call costs about 100 µs** of wall time and 80–130 µs of host CPU.
- **Compiling a model costs about 0.7 s**, and one process holds at most about 119 compiled models.
- **Counts are exact only to 2048,** and the shared dimension of a product is limited to 16,384.
- **There is no gather, sort or `select`.** Those run on the CPU.
- **Host CPU load does not slow it,** by under 10% at load 160. A busy GPU does, by 1.1–2.6×.

### The engine's real strata

These come from the Python pack, cold: 486 files, 107,714 nodes, after the CPU fixes in 0095's "Found on the way"
(profile of 2026-11-09).

- **Joins take 12.4 s of CPU** over 5,139 strata and 11,959 runs. Every stratum joins.
  - Comparisons are in strata that take 8.1 s, negation in 6.8 s, computed terms in 3.2 s, recursion in 2.6 s,
    aggregates in 1.4 s and text tests in 0.4 s.
- **Every stratum is small.** No run of a stratum takes 100 ms. By CPU per run:

  | Per run | Strata | CPU |
  |---|---|---|
  | under 0.1 ms | 2,477 | 0.17 s |
  | 0.1–1 ms | 1,942 | 1.58 s |
  | 1–10 ms | 604 | 4.87 s |
  | 10–100 ms | 116 | 5.81 s |

- **Outputs are large next to inputs.** The relational lab (0095 step 0) captured the 144 strata of more than
  3·10^5 planned units. Their sources total 12.4 M rows, and they write 6.7 M rows. 26 of them write more rows than
  they read.
- **A typical capture** joins two small tables on one column and writes every pair: 63 K rows and 2.6 K rows become
  617 K. Its reader is the next stratum.

What that means for the lanes:

- **A device call per stratum loses on this pack.** A GPU submission costs about 0.13 ms, and an ANE call about
  100 µs. Either one costs more than the whole stratum for 2,477 strata, and is a large share of it for 1,942 more.
  So the devices pay only with device waves (0095 §5), or with work batched across documents.
- **The spikes' corpus is about 200 times the pack's**, and that is where the device forms were measured. The
  data-layer repository's profile (0095 step 0) decides how much device work there is at scale. It has not been run.
- **The CPU forms pay at both sizes.** They have no dispatch cost.

## Decision (proposed)

### 1. Four more physical forms in the one plan

0095 §4 gives each join step a form: tile, hash or index. These four join them. The planner recognizes them from the
spec and records them in the plan, and every lane implements them.

| Form | Recognized when | What it does |
|---|---|---|
| `count table` | An equality join or a group whose key is a per-document code (a local name id, a kind, a node's low 16 bits: 0095 §6), within one document | A table indexed by the code: counts, or the first rows, per key. No hash and no row per pair. |
| `count only` | The reader of a step's rows needs only their number or their existence: `count { … } = 1`, a negation, a semi-join | The step's rows are counted, not written out. |
| `bit product` | Set-valued rows tested for overlap: the prefilter's candidates, co-occurrence, many-to-many existence | Rows as bitsets; AND and popcount, as a matrix product of bits |
| `closure` | A linear recursive stratum that is a transitive closure: `r(x,y) :- e(x,y). r(x,z) :- r(x,y), e(y,z).` | Condense the strongly connected components, then propagate bit rows over the components' DAG in topological order |

- **Each form is exact.** It gives the rows the current forms give, or their counts where `count only` applies. The
  relational lab checks every form row for row against its captured strata.
- **A form is a planner's choice, priced like the others.** `count table` is taken when the codes fit the table:
  16 bits, or 2,048 entries within a threadgroup. `bit product` is taken when the sets are dense enough for bits to
  beat sorted runs, which breaks even at about 1/16 (0045).
- **`count only` is the bandwidth lever.** It removes the row writes, which are what cost the most in the spikes.
  Demand (0048) already measures what a reader asks of a relation. This adds whether the reader needs the rows at all.

### 2. Placement: four lanes, priced per form, with the lanes' load

0095 §8 places a stratum on the CPU or the GPU, from one price per unit of work. That becomes:

- **Lanes:**
  - `cpu`: the cores, NEON;
  - `sme`: one worker per CPU cluster, since a cluster's cores share one SME unit;
  - `gpu`: device waves;
  - `ane`: one queue of compiled models.
- **A price per form and lane, measured.** The spikes give the first prices. The relational lab replaces them with
  prices measured on captured strata. The GPU's assumed price (2 ns a unit) goes.
- **A stratum runs on a lane only if the lane has every form it uses**, as in 0095 (`DEVICE_FORMS`, then one list
  per lane).
- **A parity band for the low-power lanes.** The ANE is chosen when its price is within a factor `B` of the best
  lane's, starting at `B = 3`. GPU work is chosen within the same band when the CPU has other work queued. Near
  parity, offloading wins: it frees the CPU and GPU, and the ANE draws less power.
- **Load counts.** A lane's price includes what is already queued on it:
  - while a GPU wave is running, ANE-eligible work goes to the ANE;
  - while the CPU's workers are all busy, SME and ANE work go ahead of more CPU work.

  The planner places a wave's strata together, not one at a time.
- **The profile reports, per lane:**
  - what ran there;
  - the CPU and GPU time it freed;
  - the time it lost against the fastest lane;
  - energy, where it can be measured (open question 2).

  The "Placement" section keeps naming the forms each lane lacks.

### 3. The Neural Engine's product path

The spikes built Core ML models with coremltools, a Python tool. The product never depends on CPython. So:

- **A Rust writer of Core ML ML Programs.**
  - An ML Program is a protobuf (`Model.proto` with a MIL program, schema published under BSD-3 in Apple's
    coremltools repository) and a weight file.
  - The engine writes the few graph templates the forms need: a bit product as a matmul against a 1-bit palettized
    constant, and an equality count.
  - The templates come from the plan's form and shape, not one per case.
- **It is compiled at run time on the machine** (`MLModel.compileModel(at:)`), and the compiled model is cached by the
  constant's digest. Placement is checked when the model is compiled (`MLComputePlan`):
  - an operation the ANE would not run is reported as a form the ANE lacks;
  - nothing is silently moved to the CPU.
- **Only fixed relations become constants.** A relation that changes per run is an fp16 input, which costs 16× the
  bytes of packed bits. The constant is something that stays the same across many runs:
  - a pack's rule vocabularies: the prefilter's rules × documents, with the rules as the constant;
  - library facts held once (0089), probed by each document's batch.
- **Shapes stay inside the limits.** The planner splits the shared dimension at 16,384 and keeps counts under 2048,
  or routes the work elsewhere.
- **Calls are batched.** At about 100 µs a call, the ANE takes a wave's worth of documents in one call, never one
  stratum's.
- **The compile cost is paid once.** At about 0.7 s a model, a model is built only for a constant that is used across
  runs. It is kept in the store, keyed by the pack's or library's digest.

### 4. SME kernels

- **Hand-written shared kernels**, as 0095 §3's MSL kernels are: the bit product (BMOPA), and the dense counts behind
  co-occurrence and closure tiles.
- **Written in C with ACLE intrinsics and built by `cc`**, which the engine already uses. Cranelift has no SME, and
  inline assembly in Rust is the alternative if C is not wanted.
- **The lane exists only where the CPU has SME2** (`hw.optional.arm.FEAT_SME2`). Elsewhere placement does not offer
  it. That is placement, not a fallback.
- **Streaming blocks are long.** A call does at least about 1,000 outer products, which is 90% of peak. A cluster's
  SME worker takes the bit-product strata of a wave in turn.

### 5. GPU

0095 stands as it is, with two additions:

- kernels for `count table` (the table in threadgroup memory: 2,048 u16 counters are 4 KB, within 0095 §3's budget),
  `closure` (level by level over the components' DAG) and `bit product` (AND and popcount on the threads);
- the GPU's matrix units are used only for products of more than about 80–200 int8 operations per byte. Boolean work
  stays on the threads, which read 8× fewer bytes.

## Order of work

0. **Done.**
   - The spikes (`tools/matrix-offload/`), with every kernel checked exactly.
   - The relational lab: 144 Python-pack strata captured and replayed row for row.
1. **The data-layer repository's profile and lab capture** (0095 step 0, its last part). This is measurement only. It
   says how much of a run each form and each lane would take at scale.
2. **The CPU forms in the native engine:** `count only`, `count table`, `closure`, then `bit product` on NEON.
   - Gates: lab replay, row for row, and identical findings on the Python pack.
   - They pay on every machine, under both bindings, at the pack's size.
3. **Placement by lanes** (§2), with the profile's per-lane report. Until a lane has kernels, it is priced but never
   chosen, as the GPU is today.
4. **The GPU path, 0095 steps 1–8,** with the new forms' kernels placed where those steps reach them.
5. **The SME lane:** the bit-product kernel and its worker per cluster.
6. **The Neural Engine lane:**
   - a. the ML Program writer, with one template, checked against coremltools' output for the same graph;
   - b. the prefilter's rules × documents, with the pack's rules as the constant;
   - c. library facts held once, probed by documents' batches.

Each step reports:

- CPU time (user + sys) and the machine's load;
- device time per lane;
- the CPU and GPU time each lane freed.

## Gates

- **Row for row** on the relational lab's captures, for every form on every lane.
- **Identical findings** on the Python pack, then on the data-layer repository's 142 checks.
- **No silent rerouting.** A lane that cannot run a stratum's form is named in the profile, and the stratum is placed
  elsewhere by the planner.

## Open questions

1. **The parity band.** `B = 3` is a starting value: is the ANE worth up to 3× its best competitor's time? The band
   could be set per binding (on battery, or on a server).
2. **Measuring energy.** `powermetrics` reports per-unit power but needs root. Without it the profile reports time
   freed, not energy.
3. **Where compiled models live.** The plan here is the content store, keyed by digest. Their size is about 1/16 of
   the fp16 constant.
4. **Whether `count only` is the planner's choice or the language's.** It can be inferred from the reader, or a rule
   could say it (`count { … }` already does).
