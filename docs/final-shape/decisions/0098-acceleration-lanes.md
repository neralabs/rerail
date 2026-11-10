# 0098: Accelerating relational work: forms first, then four lanes (CPU, SME, GPU, Neural Engine)

Status: draft (2026-11-09). It extends 0095. The executor, the four lanes and the first engine forms are built ("Built", below).

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

### 6. The executor: launches batched per lane, lanes run together

A **launch** is one kernel over plain columns: a bit product, a count table (counts or pairs), a histogram, a
closure (`rerail-accel`'s `Kernel`). Each has one meaning, the CPU reference's, and every lane's output must equal
it. A **batch** is a wave of launches that do not read each other's outputs. The executor (`rerail_accel::exec`) runs
a batch in three parts:

1. **Placement.** Launches, largest first, each go to the lane `price::choose` picks, counting what the batch has
   already queued on each lane. A lane with nothing queued pays its submission's fixed cost. Among the lanes that
   finish within the parity band of the first, the least energy wins. A launch may be pinned to a lane.
2. **One submission per lane.** Each lane gets its share at once:
   - the cores: one parallel pass on the rayon pool;
   - SME: the batch cut into tiles, taken by one streaming worker per cluster;
   - the GPU: one command buffer, with barriers only between stages that read what an earlier one wrote;
   - the Neural Engine: one batched prediction per model.

   Batching is what makes the fixed costs pay: a lane's submission is paid once a wave, not once a launch.
3. **Concurrency.** The lanes' submissions run at the same time, each on its own thread. The batch ends when every
   lane is done.

Two more rules:

- **No fallback.** A launch no present lane can run is an error that names it, and so is a lane's failure.
- **A report per batch**, for the profile: each lane's launches, wall time and estimated joules; the CPU time the
  other lanes freed; the time lost against the fastest lane; and the process's CPU over the batch.

**How the engine feeds it.** A stratum whose form the planner recognized submits its launches through the
executor of its run's binding (`logic::lanes`):

- **CPU binding:** the cores and SME. SME is part of the CPU.
- **Metal binding:** every lane.

A batch is a dependency level's launches across strata (device waves, 0095 §5: "Device waves for kernel
launches", below). The executor took them without any change to its interface.

## Answered (the user, 2026-11-09)

1. **"Design a proper executor, which will be able to batch launches"**: §6.
2. **Power: "don't measure power, you can estimate it from information on the internet."** Busy power per lane,
   from published figures (`price.rs`):
   - P-core: 6 W (Notebookcheck);
   - SME: 12 W (taken as two P-cores; no published figure);
   - GPU: 45 W (Notebookcheck's listed figure);
   - Neural Engine: 3 W (community `powermetrics` readings of 2–3 W sustained).
3. **The parity band: "sure, we can adjust later."** `Prices::band`, 3 to start.
4. **Compiled models: "facts if JIT compiled and part of the executable if AOT compiled."**
   - **JIT:** a model compiled at run time is kept in the content store's `accel_models` table, under the digest of
     its template and its fixed relation, and read back by later runs.
   - **AOT:** a model listed in `accel/models/aot.list` is compiled by the build (`xcrun coremlcompiler`) and
     embedded in the executable.
   - **Lookup order:** the executable, then the store, then a compile.
5. **Count only: "infer it if we can."** Inferred (`simplify::project_unread_columns`; see "Built").

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
   - Begun: a stratum's seed joins (equi-joins, negation, existence, the filters after an atom) run on the GPU
     when placement chooses it, one submission per stratum (0095, order of work, step 4; "Built" below).
5. **The SME lane:** the bit-product kernel and its worker per cluster.
6. **The Neural Engine lane:**
   - a. the ML Program writer, with one template, checked against coremltools' output for the same graph;
   - b. the prefilter's rules × documents, with the pack's rules as the constant;
   - c. library facts held once, probed by documents' batches.

Each step reports:

- CPU time (user + sys) and the machine's load;
- device time per lane;
- the CPU and GPU time each lane freed.

## Built (2026-11-09)

All of it was checked exactly against the CPU reference. Times are from release builds of `rerail-accel` at load
48–123, except where a debug build is named.

- **`rerail-accel`**, a crate of the workspace:
  - the kernels and their CPU reference;
  - lane detection: SME2 through `sysctl`, one SME unit per cluster;
  - prices per form and lane, from the lanes' own measurements in a release build (`price::bench`), and power per
    lane;
  - the executor.

  34 tests pass. One of them runs a single wave on all four lanes at once; placed by price, its small launches go to
  SME and the cores.
- **The SME lane:** the bit product on BMOPA, in C with ACLE, about 3.7·10⁻¹⁴ s a bit-operation and 1 µs a
  submission.
- **The GPU lane:** all four forms, a batch as one command buffer. Each submission costs about 0.2 ms of wall time
  (80–100 µs of host CPU), of which 8 µs are on the device.
- **The Neural Engine lane:**
  - **The model writer:** ML Programs written from Rust, byte for byte what coremltools writes for the same graph,
    with the fixed relation as a 1-bit constant.
  - **Compiling and running:** compiled through the Objective-C runtime, every model's placement checked with
    `MLComputePlan`. A model not wholly on the engine is refused, with its operation names.
  - **Counts above 2,048:** the shared dimension is split into pieces of at most 16,384 bits and 2,048 ones per
    constant row, so every partial count is exact.
  - **Speed:** about 1.1–4.4·10⁻¹³ s a bit-operation in the prediction calls, and 2.4·10⁻¹³ for a whole
    submission at W5's size.
  - **IOSurface inputs and outputs.** A tile's rows are written in place into IOSurface-backed arrays, and the
    engine writes its counts into IOSurfaces given as the outputs' backings, so Core ML copies nothing on the host.
    The surfaces are pooled by shape and reused across submissions. The host's own part (expanding bits to fp16,
    summing the pieces) runs on the lane's thread. Host CPU a submission, user + sys, same process and model:
    - W1: 12.0 ms on plain arrays (the old path), 5.3 ms on IOSurfaces; Core ML's share 3.3 ms, then 1.1 ms.
    - W5: 14.3 ms, then 8.8 ms; Core ML's share 2.5 ms, then 0.35 ms.
    - A shape past IOSurface's limits keeps the plain arrays.
  - **The host's work overlaps the predictions.** On IOSurfaces a submission's tiles are a pipeline over two sets
    of surfaces. While tile k is predicted on a predictor thread, the lane's thread decodes tile k−1's counts and
    expands tile k+1 into the other set.
    - Two bounded channels carry the tiles between the threads, and at most two are in flight. The lane's thread
      waits only for a prediction it needs, and for at most 120 s.
    - `RERAIL_ANE_OVERLAP=off` runs the tiles one after another.
    - The lane reports its phases through `Backend::submit_timed`: `encode` (the expansion), `device` (the
      prediction calls), `decode` (the counts read back and summed), and `worker_cpu` (CPU off the lane's thread:
      the predictor's and Core ML's). With the overlap these run at the same time, so they sum to more than the
      wall time.
    - Measured by `price::bench::ane_overlap`, a release build at load 130–158. Each row is ms a submission: the
      phases' medians, then the best wall and CPU (user + sys) of two rounds each way.

      | workload | overlap | encode | predict | decode | wall | CPU |
      |---|---|---|---|---|---|---|
      | W1, 7 tiles | off | 0.69 | 2.05–2.10 | 3.1–3.2 | 5.6–5.8 | 4.5–4.6 |
      | W1, 7 tiles | on | 0.67–0.68 | 2.1–2.2 | 3.2 | 4.1–4.2 | 5.0 |
      | W5's constant × 8,192 rows, 4 tiles | off | 6.7–8.5 | 22–25 | 21–27 | 48–59 | 28–34 |
      | W5's constant × 8,192 rows, 4 tiles | on | 9.6–11 | 23–24 | 57–83 | 37–58 | 30–35 |

    - W1's wall fell by about a quarter, and its CPU rose by about 0.4 ms, the second thread's handoffs.
    - On W5, the best wall fell too, but the decode's wall grew 3–6× while its CPU stayed at 24–27 ms. So the decode
      is off the CPU, not locked: timing the surface locks found 3 slow ones in 1,012. At this load the median wall
      is no better than serial. It wants a run on a quieter machine.
- **In the engine:**
  - **`count only`, inferred.** A derived relation's column that no reader reads, positive or negated or in a count
    compared with 0 (the converter's "no such row"), is dropped: the relation's rows are their projection.
    - On a 59-program slice of the Python pack it projected 62 relations (83 columns), with identical outputs.
    - With the count case, on the 8 programs that read `bound_in` (486 files): 47 relations lost 74 columns,
      among them the `rr_agg*` helpers. Outputs are identical with it on and off. Local CPU was 14.2 s on and
      14.5 s off (user + sys, load 9–19): no gain at this size.
    - The count case first left the count's own atom at the old arity, and the planner panicked. A test now covers
      it.
    - `RERAIL_PROJECT_COLUMNS=off` turns it off.
  - **`closure`.** A component whose one recursive probe walks one column along an edge relation (`seed ∘ E⁺`) runs
    the edge's join once per distinct node. No path leaves a weakly connected component of the reach graph, so each
    component's reach is found on its own:
    - a component whose every node has one successor (a tree climb) by walking its chains;
    - components of up to 16,384 nodes packed into pieces of about 1,024 nodes, one `Closure` launch of the
      executor per piece. A piece's bit rows grow with its nodes squared, so packing keeps the bits near the graph's
      nodes times 1,024, not its nodes squared;
    - a component past 16,384 nodes, or past the closure's budget of 2³⁰ bits, by a search on the host from each of
      its start nodes, once per start.

    So no closure runs semi-naive any more, except when its edge search fails. `RERAIL_CLOSURE_FORM=off` turns the
    form off. On the same slice, outputs are identical with it on and off (local CPU 14.2 s on, 13.9 s off: noise).
  - **`count table`** (`setfix::table`). A count step over an input walks the matching rows at every binding that
    reaches it. Once its walks have read as many rows as building a table would, the step reads a table instead.
    So it never costs more than about twice the walks alone.
    - **Counts per row:** when the key comes whole from the join's first step, a scan, every scan row gets its count
      from one `CountTable` launch (counts only, no pairs). The step then reads its row's count by position.
    - **Histogram:** otherwise the relation's counts by key, from one `Histogram` launch. The table is the
      relation's, like its indexes, so every step in any stratum that counts it by the same columns reads it.
    - **Codes:** keys get 16-bit local codes within partitions (0095 §6). A partition here is a hash of the key, not
      a document, so each holds at most 2^16 distinct keys whatever the document's size.
    - **On the slice:** `up_once`'s count of `binding` and its annotation copy take counts per row, and `rr_agg67`
      a histogram.
    - **Gate:** outputs are identical with it on and off. Local CPU was 18.6 s both ways (load 106–110): these counts
      are small at the pack's size.
    - `RERAIL_COUNT_TABLE=off` turns it off.
  - **Device waves** (0095 §5): "Device waves for kernel launches", below.
    - **Count tables.** When a level's first stratum runs, the level's count tables are built as one batch: those
      an earlier chunk built and whose relations the run holds. So each lane gets one submission per level. On the
      slice's second chunk, a level of 15 strata built its one table that way.
    - **Closures.** A level's closures prepare their launches, wait, and run as one batch when the level ends.
  - **`bit product` in the prefilter** (`prefilter::gates`). Each plan's gate, an `And` and `Or` of terms, goes to
    disjunctive form, up to 256 conjunctions. Then one launch serves every plan of the batch:
    - the batch's files are rows of bits over the terms, and the conjunctions are rows of bits marked as a fixed
      relation, which the Neural Engine holds as a constant;
    - a pair is kept when the file has all of the conjunction's terms (`min`).

    A gate's files are then the uncovered files plus the files of its conjunctions, exactly the set form's. A test
    checks that on 200 random gates. A larger gate keeps the set form. `RERAIL_PREFILTER_BITS=off` turns it off.
  - **The profile** names every form a stratum took: closures by chains or bit rows, count tables by relation and
    columns, and each level's device wave.
  - **Model facts:** the content store's `accel_models`.
  - **The lanes per binding:** `logic::lanes`.

### Joins on the GPU from the engine (2026-11-09)

The GPU lane's first engine form outside `rerail-accel`: a stratum's seed joins as stages over binding tables
(`setfix::device`, `metal_join`; 0095's order of work, step 4, has the design). It goes through the native engine,
not the executor, because its unit is a stratum's rules, not a launch; a dependency level's strata would be one call
of the same function, so device waves need no new interface.

- **Placement** has join, negation and comparison as the GPU's forms (`DEVICE_FORMS`), priced from the host's
  measured cost per submission, rule, stage and sort. A stratum whose shape the device's run does not take is
  refused and named in the profile with its CPU.
- **Python pack** (debug build), findings identical to the base binary. Paired under the Metal binding, device
  joins on and off at the same time (load 13–20):

  | | joins on the CPU | joins on the device |
  |---|---|---|
  | Local phase, user + sys | 125.5 + 5.5 s | 117.4 + 6.1 s |
  | Retired instructions (process) | 2,731 G | 2,596 G |
  | The 345 strata the GPU ran (756 runs) | 25.7 s CPU | 15.9 s CPU, 1.0 s GPU |

  - The host's part of the device's work was 5.1 s: 0.8 s encoding, 1.3 s uploads, 0.06 s waiting and 2.9 s
    putting 30 M rows into the store. Rows back are the floor, as §"The engine's real strata" found: bandwidth
    goes on rows written out.
  - Aggregates' inputs gain most (`rr_agg36` 1.64 to 0.56 s, `rr_agg239` 0.49 to 0.03 s).
  - The device's buffers share the process's 256 MB with the syntax side's tapes; the joins take what is left
    and name a stratum refused for space.
- **Not yet:** level batches; uploads that outlive a chunk's relations (a library relation is uploaded again each
  time the engine rebuilds it); the host's per-rule cost.

### Where the time goes: planning, launching and execution (2026-11-09)

The profile's `== Acceleration (decision 0098)` section shows each form's time, phase by phase:
- **Host phases**, timed on the engine's thread (wall, and that thread's CPU):
  - plan: recognizing the form;
  - encode: the kernel's inputs (a closure's edge search, a table's key codes, the prefilter's bit rows);
  - decode: the kernel's output into the engine's rows or tables.
- **Per lane**, from the executor's report:
  - launches and submissions;
  - the submission's wall, and the lane thread's CPU;
  - the backend's own encode, device and decode times (`Backend::submit_timed`, `Phases`): GPU device time
    from the command buffers, SME's streaming workers, the Neural Engine's predictions;
  - workers' CPU (the rayon pool for a CPU launch, SME's spawned workers, the Neural Engine's predictor thread).
- **The executor's own:** placing, and spawning and joining lane threads. A batch on one lane now runs on the
  caller's thread, without a spawn.
- **Counts:** nodes searched, rows made, and the rows walked before a table paid for itself.

The whole Python pack (534 programs, 486 files, CPU binding, load 100 to 120), CPU as user + sys:

| | CPU |
|---|---|
| the run, every span | 293 s |
| joins, file-local part | 110 s |
| moving rows to the cross-file part | 39 s |
| statistics phase | 24 s |
| parsing | 23 s |
| planning strata | 22 s |
| opening inputs, building indexes | 16 s |
| **every form's host phases** | **6.3 s** |
| closure: encode (its edges, 3.0 M nodes searched in 159 closures) | 5.2 s |
| closure: decode (680 k rows made) | 1.1 s |
| count tables: encode and decode (6 histograms) | 0.02 s |
| **every kernel** (4 closures by bit rows, 6 histograms; wall) | **0.3 s** |
| the executor's placing and threads | 0.2 ms |

What it shows:
- **The bottleneck is not in the kernels.** It is not in launching them either. The host work around the
  kernels costs 20 times what the kernels do, and the closure's edge search is most of it.
- **Most closures launch nothing.** 140 of 159 are chains (tree climbs), which the host walks, and 15 more fell
  back to semi-naive past 16,384 nodes (`prog_ast_parent`, `prog_ast_up` and 8 more). So only 4 used a kernel. All
  19 now run in pieces, in device waves (below).
- **The closure's real cost is its expansion.** It makes 680 k rows, which its readers then join. That is
  decision 0097's case for not materializing: keep (row, start node) and look the reach up in the reader.
- **The forms are 2% of the run under the CPU binding.** In that binding every launch stays on the CPU, since
  closures and histograms have only CPU and GPU kernels.
- **The larger prize is the strata.** The placement planner (0095) finds 553 strata whose work would be cheaper
  on the GPU: 84 s of CPU against an estimated 0.4 s on the device. That needs the Metal binding (`--metal`), and
  their forms on the device.
- **Under the Metal binding** (`--metal`, the same pack, load 94 to 124), findings are identical and nothing goes
  to the GPU:
  - The forms' launches are too small for the GPU's fixed cost, so the prices keep them on the CPU (kernels 64 ms).
  - None of the 553 strata can run there either: each lacks a GPU join (78.9 s CPU). Many also lack negation
    (46.0 s) or comparison (49.0 s).
  - Local CPU was 197.4 + 7.6 s against the CPU binding's 206.7 + 10.3 s, which is noise at this load.

  So the expected gain waits on 0095's device join (sort and probe), negation and comparison, not on these forms.
- **Fixed on the way:**
  - The closure searches its edges in one join context for every node, not one per node: its encode fell from
    1,055 to 467 ms CPU on the 8-program slice.
  - Its rows are pushed once, with no fresh-row buffer hashed first.

**After the merges (2026-11-10).** The same pack, cold, CPU binding, profiled, one run each. The merged binary has
closure waves, factorized exports, the front-end and statistics work, observed step counts, device strata joins, kept
cross-file strata, the inliner fix and streamed prepared parts. Load: base 9 to 12, merged 14 to 21. Instructions are
`time -l`'s retired count. The merged run was its binary's first profiled run, so it also measured the roofline
(about 62 G instructions and 0.7 GB, after the run's work; the base's roofs were cached).

| | base | merged |
|---|---:|---:|
| process CPU (user + sys) | 191.9 s | 158.6 s |
| instructions retired | 3,026 G | 2,403 G (about 2,341 G without the roofline) |
| peak footprint, the profile's own | 1,429 MB | 1,375 MB |
| joins, file-local part | 65.4 s | 54.9 s |
| moving rows to the cross-file part | 26.1 s | 22.4 s |
| statistics phase | 15.1 s | 12.1 s |
| parsing | 14.7 s | 10.3 s |
| planning strata | 14.0 s | 6.5 s |
| opening inputs, building indexes | 9.9 s | 8.5 s |
| guests | 8.9 s | 4.4 s |
| joins, cross-file part | 7.7 s | 6.6 s |
| measuring sources | 7.2 s | 2.7 s |

The findings are the base's except for B007: 16,033 rows become 82, which fixes the inliner's wrong renaming
(decision 0097). Every other program's output is identical.

What is left:
- The file-local joins are still a third of the run.
- Moving rows to the cross-file part is next. Its largest piece is the memo's write, 9.0 s: zstd and the entry's
  byte copies, which a debug build checks per copy.
- The peak (1.4 GB) is over the 1 GB budget. Its highest single rise was keeping the prepared program, 442 MB, half
  of it a copy of the program's facts. That copy is now gone, but the pack's peak has not been measured since.

### Device waves for kernel launches (2026-11-09)

Before this, each closure submitted its one launch alone, in the middle of its stratum, and a closure past 16,384
nodes ran semi-naive. Now a dependency level's closures launch together, and large closures run in pieces.

**The problem.** A closure's launch is known only after its stratum has done most of its work: the seeds, the edge
search over every node reached, the adjacency. Its rows are needed before the stratum can finish. So a batch of a
level's launches needs every closure stratum of the level stopped between the two, until all have prepared theirs.

**The design: a stratum suspends with its state.** It is not specific to closures. Any form that launches kernels
can use it.

- **A form runs in two halves around its launches.** For the closure, `prepare` does the seeds, the edge search,
  the components and the pieces' launches. `finish` turns the bit rows into rows.
- **A wave per thread** (`lanes::wave`). The stratum driver holds a `Suspendable` while its component runs. Inside
  one, a form puts its launches in the wave and gets tickets back. The component's run then returns a fourth status,
  "waiting on its wave", next to "done", "more waves" and "paused for a request".
- **The driver is resumable** (`Run::drive`). A native stratum is opened (its spec, plans and component), then
  driven: chunks of waves, replanning between them, the profile, the close. All the driver's state is one value
  (`Driving`). On "waiting", the driver returns it as `Native::Waiting`, and the strata loop goes on to the level's
  next stratum.
- **The flush.** The loop flushes the wave when the level ends, when a stratum depends on a waiting one, and at the
  end of the run. Every waiting launch, of every form, goes to the executor as one batch: one submission per lane.
  Then each waiting stratum resumes in order. Its run takes its outputs by ticket and finishes. A stratum that waits
  again (a second form) waits on the next flush.
- **Strata in level order.** In native runs the strata are sorted by level, a stable sort, so a level's strata
  are adjacent. A stratum's level is one more than the deepest stratum it depends on: its reads, the request it
  answers, a native operator's inputs. The dependency order alone interleaves levels depth first, so few closures
  would be pending at once. A run that is all SQL keeps the dependency order, since SQL launches nothing (and T's
  golden plans are in that order).
- **What a waiting stratum may still read.** Nothing outside its own component. It read its inputs before it
  waited, so the loop may drop the inputs it read, and release the relations it read, while it waits. Its own
  relations join the native ones when it finishes. Those that no stratum reads are released or read out at the next
  stratum, as they would have been at the stratum after them.
- **Where no caller can wait** (a component run through SQL, the lattice fixpoint, a test), the form runs its
  launches at once, as a batch of their own. `RERAIL_WAVES=off` does that everywhere. It is a measurement switch.
- **The profile** counts batches by their launches, the batches each dependency level ran, the waves flushed, and
  for closures the chains, pieces and host walks.

**Pieces instead of the semi-naive fallback.** A closure's reach graph is split into its weakly connected
components ("Built", above). On the pack, 15 closures fell back past 16,384 nodes. They are 10 relations, some in
both chunks: `ast_parent`, `ast_up`, `literal`, `p202_await_ok`, `p362_parsed_within`, `runs_at_import`, `up_fn_of`,
and `rr_in_annotation_0`'s copies of three of them. All 15 run by pieces now. None of their components is past the
limit, so on the pack no closure runs semi-naive and none is walked on the host. A test covers a component past
the limit (host walks), with 100 chains and 200 small cyclic components packed into several pieces. Its rows equal
semi-naive's and a plain search's, run alone or inside a wave.

**A workload where waves matter** (`fixtures/python/logic/closure-waves/`): six programs over the ruff fixtures,
with a closure each over a tree or a graph that forks:

- the descendants of each def, each class and each call's arguments;
- statement successors through ifs;
- inheritance within a file;
- the call graph within a file, a level above the descendants of each def it reads.

The 486 files are 2 chunks, so 14 closures run: 11 by pieces, 3 by chains alone. Per level, with `RERAIL_WAVES=off`
for the comparison. Three runs each, at load 94–120; CPU is user + sys:

| | waves | batches | batches in a level | kernels' wall (ms) | kernels' CPU (ms) | host phases' CPU (ms) |
|---|---|---|---|---|---|---|
| CPU binding | off | 11 | 3–4 in two levels, 1 in three | 455, 190, 159 | 35, 35, 35 | 166, 164, 167 |
| CPU binding | on | 5 | 1 in each of five | 211, 107, 71 | 35, 32, 37 | 155, 151, 165 |
| Metal binding | off | 11 | 3–4 in two levels, 1 in three | 1,687, 128, 169 | 41, 35, 34 | 158, 160, 166 |
| Metal binding | on | 5 | 1 in each of five | 242, 102, 120 | 33, 34, 34 | 158, 162, 160 |

- **The level of four closures** (descendants of defs, classes and calls, and the call graph's def bodies) goes
  from 4 batches to 1 in each chunk: 44 launches in one chunk, 19 in the other. The 66 launches went into 5
  batches with waves, and 11 without.
- **The kernels' wall time falls by about half** (best runs: 71 against 159 ms, and 102 against 128 ms): one
  parallel pass over a wave's launches, not one per stratum. Their CPU is the same. Wall times at this load are noisy.
- **The host phases do not change.** The edge search (about 75 ms) and the decode (about 77 ms) are each stratum's
  own work, whether it waits or not.
- **Under the Metal binding every closure launch stays on the cores.** By the measured prices, a closure costs
  7.5·10⁻¹⁰ s a unit on one core and 1.7·10⁻¹⁰ s on the GPU. Spread over 18 cores, the cores finish first and use
  less energy, whatever the batch's size. So a wave saves the GPU's submissions only once a form's GPU kernel beats
  the cores.
- The findings are identical across the four configurations.

**The pack and the slice** (CPU binding):

- **Findings are identical** to the binary before this change: the 8-program slice under both bindings, and the
  whole pack once.
- **The pack**, base then new, at load 98–154:
  - CPU: 298.9 + 14.0 s against 296.2 + 13.5 s;
  - the closures' host phases: 6.22 s against 6.37 s;
  - peak RSS: 1,448 MB against 1,565 MB. The run's own phase table puts the difference already in its first phase
    (`combine`, 423 against 518 MB), before any stratum runs, so it is not clearly the level order. A quiet paired
    run would settle it.
- **Pack waves.** 19 closures ran by pieces (37 launches), 140 by chains alone. The pieces' kernels took 272 ms of
  wall in all, against 45 ms for the base's 4 launches: the 15 closures that ran semi-naive before now launch.
  - 10 waves held the 19 waiting strata.
  - The executor ran 15 batches over 12 levels: 10 levels ran one batch, one level two, and one level three or four.
    The batches past the waves are count tables: 2 built ahead for a level, and 3 that a step built once its walks
    paid for one, mid-stratum, which do not wait for the wave yet.
- **The slice** has one closure by pieces, which before fell back to semi-naive.
- **By retired instructions** (the run's report; steadier than CPU at this load), after merging the base branch's
  latest, paired runs each way, the first run of each binary left out:
  - the slice: 232.50 and 232.74 G for the base, 232.77 and 232.48 G with waves and pieces (equal within 0.1%),
    findings identical;
  - the closure-waves workload: 22.63 and 22.72 G for the base, 22.02 and 21.95 G with waves and pieces (−3%),
    21.90 and 21.96 G with `RERAIL_WAVES=off`. So waves change where the kernels' wall goes, not the work done; the
    3% is the pieces.

**Not built yet:**

- **Count tables a step builds mid-stratum** still launch alone. A step's table is known only once its walks have
  paid for it, in the middle of a join. Waiting for the wave there needs the join itself to stop and resume, which
  the two halves of a form do not cover.
- **The Neural Engine's overlap at large tiles, measured quietly.** At W5's tile size, under heavy load, the overlapped
  decode is descheduled. Its median gain is unproved until a run at low load. The plain-array path, for shapes past
  IOSurface's limits, is not pipelined.

## Gates

- **Row for row** on the relational lab's captures, for every form on every lane.
- **Identical findings** on the Python pack, then on the data-layer repository's 142 checks.
- **No silent rerouting.** A lane that cannot run a stratum's form is named in the profile, and the stratum is placed
  elsewhere by the planner.

## Open questions

1. **The band per binding:** on battery, or on a server, the band could differ.
2. **What is not materialized:** decision 0097 (draft) takes the user's question of relations that expand and then
   narrow.
