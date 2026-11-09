# 0100: One executor for a chunk's strata: levels sent to every lane at once, relations kept on the GPU

Status: draft (2026-11-09). It builds 0095 §5 (device waves) on the executor of 0098 §6. The order of work is
below; steps 1–3 are done.

## Intro

**The problem.** The engine runs a chunk's strata one at a time, on the host. Device work exists, but each piece is
sent on its own:

- A join run on the GPU by sorting the probed relation and probing it costs about 100 µs of host CPU per sort and
  55 µs per probe, whatever their size. Nearly all of that is the submission: about 20 µs of process CPU for an
  empty command buffer, then 1.5 µs a dispatch.
- On the Python pack's 232 measured joins (5.6 M pairs), the GPU's joins sent one by one cost 128 ms of host CPU.
  The CPU's radix sort join costs 96 ms. One at a time, the device cannot win on host CPU.
- A relation the GPU sorted is thrown away when its join ends, though later strata of the chunk read it again: 94
  of 138 sorts were of a relation sorted before.

Decision 0098 built an executor (`rerail-accel`) that already places a batch of launches on four lanes (the CPU's
cores, its matrix unit SME, the GPU, the Neural Engine) and sends each lane its share as one submission. But it
keeps nothing on the GPU between batches, it has a Metal device and queue of its own beside the engine's, and the
engine gives it one stratum's launches at a time.

**What changes.**

- **One executor.** Every launch the engine sends to any lane goes through `rerail_accel::Executor`. There is no
  second scheduler.
- **One Metal manager for relational work**, in `rerail-accel`: the device, one queue, the pipelines, the pooled
  buffers, and the engine's launch budget on every commit.
- **A level of strata is one batch.** Strata at the same depth of the dependency order do not read each other. Their
  device launches go out together: one submission per lane per level.
- **Lanes run while the host works.** A submission returns at once. The level's CPU strata run while the GPU and the
  Neural Engine work, and a stratum waits only for the launches it reads.
- **Relations stay on the GPU across the chunk's levels.** Columns are uploaded once per chunk, a relation's sorted
  form is kept, and an output a later level reads is never read back in between.

**A concrete case.** One of the pack's chunks has a level of 15 strata (0098). Suppose each probes a relation an
earlier level made. Today that is 15 host runs, or on the device 15 sorts and 15 probes, each sent alone. Under this
decision it is one command buffer: the sorts not already resident, then the probes, with one barrier between them.
The level's CPU strata run meanwhile.

## Prior work consulted

- **ast-index-spike design index, §19 GPU/SIMD.** Nothing was designed or measured for a GPU. Set-based decoding is
  a prefix sum (D S053), and row bookkeeping dominates per-touch cost (D S152).
  - **Agrees.** Every device kernel here is columnar: counted, then scattered.
- **Design index §3–4.** Mis-estimated intermediates pick bad joins (D S069, S153–S156).
  - **Agrees.** A level is placed from measured prices and counts, and the counts the replanner reads come back
    after each level.
- **Lessons ledger row 1.** One task object per tuple is fatal (1.99 GB).
  - **Agrees.** A launch is per relation and form, never per row.
- **Ledger row 26.** Acceptance is structural: the work is proportional to the inputs and results.
  - **Agrees.** Residency removes repeated uploads and sorts, and the gate counts submissions per chunk.
- **Tempo-metal's host-driven generations** stopped at 595 dispatches (0027).
  - **Agrees.** The host plans per level, not per round. Recursion inside a level stays on the work queue (0027).
- **Decisions.**
  - 0010 (waves). SQL and the planner still choose what runs. A level is the planner's order, grouped.
  - 0027 (the device work queue). **Used** for recursive strata, unchanged.
  - 0032 (resident slabs). **Used**: its budget split gives relations the run's share of the Metal buffer budget.
  - 0039 and 0040 (pre-pinned blocks; one wait per chunk). **Extended**: the pool moves into the one manager, and
    "one wait per chunk" becomes one wait per level, taken only by the stratum that needs the results.
  - 0095 §5 (device waves). **Built** here, through 0098's executor rather than beside it.
  - 0098 §6 (the executor). **Extended**: asynchronous submission, inputs resident on the device, and the sort,
    probe and tile join forms as its kernels.
- **The other work in progress**, surveyed before this record:
  - The matrix-offload work (0098, 0099) brings the executor, the lanes and the count-table waves. It is merged
    here, and its decisions were renumbered from 0096 and 0097, which were already taken.
  - The parser generators' Metal passes (`metal_block_serial`) and the transducer's label passes use the engine's
    own Metal backend. They are not relational work and are not moved.
  - A diagnostic print of refused dispatches (`metal_launch`, `RERAIL_LAUNCHES`) on another branch. It reads the
    same launch budget this record commits through, so the two fit.
  - Store-write batching and export counts by file (`plan/corpus.rs`) on two branches. These are bookkeeping
    around a chunk, not its scheduling. They touch the same file, not the same code.

## Decision (proposed)

### 1. One executor

`rerail_accel::Executor` is the engine's only scheduler of launches. The engine's own device code for relational
work (`metal_sort`, `metal_rows`) becomes kernels of the executor's GPU lane. Every kernel keeps one meaning: the
CPU reference's. Each lane's output must equal it, and the tests check that.

### 2. One Metal manager for relational work

`rerail_accel::gpu` holds, once per process:

- the device and **one command queue**;
- the compiled pipelines, by name and variant;
- **the pool of pinned buffers** (0039's size classes, at most 32 MB idle). A buffer goes back to the pool only once
  every command buffer that reads it has completed;
- **a commit hook.** The engine sets it to its launch budget (`metal_launch::check_submission`), so every command
  buffer counts against the one budget and is refused by it.

The transducer's Metal backend (`metal_backend`, the label passes, the parser's passes) keeps its own state. It is
another kind of work, and this record does not move it.

### 3. Submissions do not wait

A lane's `submit` returns a pending result. The executor's `begin(batch)` places and submits a batch on every lane
and returns at once. `wait` on a launch blocks only until that launch's lane is done.

- The GPU commits its command buffer with a completion handler, and waits on that handler, not by polling.
- The cores run their share on the thread pool. SME and the Neural Engine run on their own threads, as now.
- A failure of any lane is an error that names the launch, and there is no fallback (0098 §6).

### 4. Relations resident in a chunk's space

A **space** (`rerail_accel::Space`) is what a chunk keeps on the GPU:

- **Columns**, by relation, version and column. An input relation is uploaded once per chunk, in 0095 §6's narrow
  form (16, 32 or 64 bits of offset from its least value).
- **Sorted forms**, by relation, version and key columns: the permutation, the run starts and the run keys of
  `metal_sort`.
- **Outputs a later level reads.** A launch can name its output as kept, and its readers then take it by name, not
  by readback.

What a level's launches read is named as `Host(columns)` or `Space(id)`. Entries go when the level after their last
reader ends (the strata loop already computes that level), and the whole space goes with the chunk. The space's
budget is the run's share of the Metal buffer budget (0032). When a level does not fit, the least recently read
entries that the level does not read are dropped first. If it still does not fit, the level is split by document,
and that is in the profile.

### 5. A level is one batch

The strata loop gives each stratum a level: one more than the deepest level it reads (built for count tables in
0098). When a level starts:

1. **Its launches are collected** from every stratum of the level whose inputs are known: count tables, histograms,
   the sorts of probed relations by their keys, and first joins (§6). A closure's launch is known only once its walk
   has found its nodes, so it is still sent alone.
2. **Placement** prices every launch on every lane (0098 §2). The join forms' prices of 0095 §8 move into the
   executor's price table, beside the other forms.
3. **One submission per lane.**
4. **The level's strata run in order on the host.** A stratum that reads a launch waits for it. The others, and
   every CPU stratum, run meanwhile.

The counts the replanner reads still come back after each level, as now.

### 6. Joins on the device, in the engine

A join rule whose first two steps read held relations on equal keys gets its pairs from the device: the right
relation sorted (kept in the space), the left probed against it, or a document tile when both are grouped by
document. The host continues the rule from its third step, with each pair's slots already bound, as the closure
form already continues a join from preset slots. Joins inside recursive strata keep the host path until the work
queue runs them (0027, O34).

### 7. The CPU's cores in parallel

Two kinds of work can run on the cores at once:

- **launches placed on the cores**, on the thread pool;
- **strata of one level** that read no SQL and share no relation they write.

Whether strata run in parallel, and with how many threads, is decided by measurement in step 6. Every stratum
reads the shared symbol table, and new symbols take its write lock, so the gain is not known ahead.

## Order of work

1. **Merge the executor's branch** and renumber its decisions to 0098 and 0099. *Done*: findings identical on the
   pack gate. On the way, inlining (0099) equated a read's term with itself when the reader had a variable of the
   inlined rule's name (`expr_name(n, n, x)` read as `expr_name($t, $n, $x)` gave `$t = $t`), which gave B007
   2,237 extra findings. Fixed, with a test.
2. **The one Metal manager:** the pool, queue, pipelines and commit hook move into `rerail_accel::gpu`.
   `metal_sort` and `metal_rows` use it, and the executor's GPU lane takes buffers from the pool instead of making
   new ones per batch. *Done* (`rerail_accel::gpu::shared`):
   - one queue for the lane and the engine's sorts and probes;
   - pipelines kept by key, for every thread;
   - one pool of shared buffers, size classes from 16 KiB, at most 32 MiB idle. The lane's buffers are zeroed on
     reuse, because its kernels count into them;
   - the engine's launch budget as the guard, checked on the thread that begins a batch;
   - the pool and pipelines released with the residency slabs.

   The lab's 232 joins (sort and probe) and 110 tiles are all exact on it, and the pack's findings are identical.
3. **Asynchronous submission:** `begin` and `wait`. *Done*: each lane has one worker thread for the executor's life,
   so a batch is handed over without spawning threads. `begin` returns once every lane has its share, and `run` is
   `begin` then `wait`. A test begins two batches, collects them in reverse order, and checks both against the
   reference.
4. **The space, and the join forms as kernels:** sort, probe and tile in the executor, with their CPU references
   (`rows_sort`) and their measured prices.
5. **Levels in the engine:** a level's launches as one batch. Then first joins from the device, with the host
   continuing from the pairs.
6. **The cores:** measure strata of one level in parallel, and decide.
7. **Gates:**
   - The pack's findings are identical, and so are the data layer's at scale.
   - Host CPU (user + sys) of the device joins is below the CPU radix sort join's, on the lab's 232 joins and on
     the pack.
   - Submissions per chunk are at most one per lane per level, and the profile shows the space's hits.

## Open

- **Count tables and the scalar summaries overlap.** The engine's per-key memo of scalar summaries answers a count
  once per key, and a count table answers it once per relation. Since the merge, a count the memo serves never walks
  enough to build a table, so the count-table test is ignored. Which of the two a level's batch should send, and
  whether the memo should read the table, is decided with step 5.

## Not decided here

- **The Neural Engine in levels.** It is a lane like the others, so a level's bit products reach it through the
  same batch. Whether any of the engine's strata are worth it is 0098's to measure.
- **Recursive strata on the device** stay on the work queue (0027) and are not batched across strata.
