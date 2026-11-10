# 0100: One executor for a chunk's strata: levels sent to every lane at once, relations kept on the GPU

Status: draft (2026-11-09). It builds 0095 §5 (device waves) on the executor of 0098 §6. The order of work is
below; steps 1–5 and 7–12 are built, and step 6 is measured.

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

### 8. Every lane at once, every job scored

The executor runs **jobs**, and each lane's jobs run beside every other lane's:

- **The cores** have one pool for the process, of `RERAIL_THREADS` threads (4 when unset). Both of the engine's
  executors (the CPU binding's and the Metal binding's) share it. Each launch placed on the cores is a job of its
  own, so a batch's launches there run on every thread at once, beside other batches' launches. A kernel that splits
  its work splits it over the same threads.
- **SME, the GPU and the Neural Engine** each have one worker thread. A batch's share of such a lane is one job, one
  submission, as in §3.
- **Host work.** `Executor::spawn` runs a closure as a job on the cores' pool, scored like a launch. A job that
  waits for another job, on the pool, runs the pool's other jobs while it waits, so jobs that wait for jobs cannot
  take every thread and stall.

**The scoreboard** (`rerail_accel::score`) counts every job when it is queued, started and finished. Per lane it
keeps:

- what is queued and running now, and the most that ran at once;
- jobs, launches and failures;
- seconds run and seconds waited in the queue;
- the price's estimate against the time taken, per lane and per form.

**Placement reads it.** A lane's starting load is the estimate the scoreboard has outstanding on it: every batch
begun and not yet finished. So a batch begun while another runs goes to the lanes the first one left idle. A lane
pays its submission once per batch that uses it, whatever other batches queued there.

`RERAIL_SCOREBOARD=<file>` writes a line per job: its lane, thread, launches, forms, units, estimate, and its
queued, start and end times. The strata loop adds the scoreboard's summary at each chunk's end.

The variable is the engine's thread setting from the rule-syntax work, where it caps the threads the process uses
(the machine's cores when unset). The executor reads the same variable, so the two agree once both are merged. The
executor's default is 4, which is what was asked for.

### 9. Placement by host CPU, and GPU jobs of a few µs

On a shared machine the cores are busy and the GPU is mostly idle. So the scarce thing is the host's CPU, not wall
time, and a device's latency costs the host nothing while it works on. Placement weighs each launch by three
numbers on each lane (`Prices::cost`):

- **finish:** when it would be done, counting what is queued on the lane, the lane's latency, and its work. The
  cores' work is stretched by the machine's load (the load average over the cores, at least 1).
- **wall:** its seconds of work there.
- **host:** the host CPU it costs there. On the cores and SME that is their work. On the GPU it is the job's
  encoding, commit and reads, plus copying host columns (about 1 ns a row). On the Neural Engine it is its
  submission.

The rule (`Prices::choose_host`) is:

1. The **deadline** is the later of two times: the parity band times the first lane to finish, and the
   batch's **slack**.
2. Of the lanes done by the deadline, the one of **least host CPU** wins, then the one of least energy.

The slack is how long the batch's caller works before it waits. A lane done by then costs the caller no wall
time. Each kind of batch (`Executor::begin_as`, such as "level joins") has its own slack: the lower quartile of
its last 32 overlaps (begin to wait). The quartile is used, not the mean, because some callers of a kind wait at
once and others much later. The mean would give the first sort the second's slack.

**Measured, not assumed.** The scoreboard keeps three moving averages per lane, and placement reads them:

- the lane's latency (the GPU's submit, schedule and wake);
- a device job's thread CPU;
- the job's host CPU over the price's estimate. This scales that lane's host estimates.

A cold job is left out of all three: one that compiled a pipeline or made a pool buffer. One cold sample would
otherwise keep the GPU from ever being chosen again.

**A GPU job's host CPU** is cut by doing the batch's work in two dispatches, whatever its number of joins:

- `sort_all`: one threadgroup per right side of at most 2,048 rows and three keys. A bitonic sort of (key, row)
  runs in threadgroup memory (20 KB), so the order is stable. It writes the runs, the order and the other columns
  gathered.
- `probe_all`: one threadgroup per 1,024 left rows of any probe in the batch. It counts its rows' pairs, reserves
  them in the batch's arena with one atomic, and writes them there. The host reads each probe's pairs block by
  block, in left row order. So no probe needs a second command buffer to learn its size. Only a probe whose pairs
  did not fit the arena runs again, into space of its exact size.
- Each job's buffers are reached through device pointers in a table the host writes. One `use_resources` call
  makes them resident.

Indirect command buffers were not needed: the descriptor tables fold the batch into two dispatches, which is
fewer calls than encoding one command per join. Larger right sides keep the radix sort, and the probes over them
join the same probe dispatch.

The rest of a job's host work:

- Left columns go up raw, 64 bits, with no span taken and no narrowing.
- The fused pipelines compile on a thread of their own when the lane starts.
- The second command buffer is made only for tile scatters and spilled probes.

### 10. Commit first, fill after

A batch's columns can be **staged** (`rerail_accel::col`). Their sizes are known when the batch is begun, and
their values are written after it.

- A `Gate` makes staged columns (`Gate::column`). Each is a pool buffer the device reads in place, and one
  `Writer` fills it. The gate opens once: when the caller calls `Opener::open`, or when the opener drops, so a
  device never waits forever on an error.
- The GPU lane's command buffer waits at its head on each closed gate's shared event. The lane encodes and
  commits at once. When the gate opens, its event is signalled, and the device starts.
- On the host, reading a staged column waits for its gate. A right side too large for the fused sort is
  narrowed from its values, so for that one the lane waits for the fill before encoding (`wait for fill`).

The level batch begins with staged columns, then fills them from the relations, and opens the gate. The lane's
encoding and commit run while the host fills, instead of after it. The submission and scheduling delay
(about 1.7 ms under load) runs while the caller goes on to its strata.

**What the explain output shows** (the profile's executor section, and the scoreboard trace):

- The measured averages per lane: latency, a device job's host CPU, and actual over estimated host CPU.
- Per lane and form, host CPU estimated against actual.
- Per phase:
  - fill after begin;
  - wait for fill;
  - fill after commit (how long the device waited for the host);
  - pipeline compile;
  - new buffer;
  - the sorts by kind;
  - second command buffers.
- Per kind of batch:
  - launches per lane;
  - launches not on their fastest lane;
  - host CPU as placed, against placing each on its fastest lane;
  - the wall time that adds;
  - the slack used;
  - the overlap and the stall;
  - batches stalled, and the longest stall;
  - the slack now.
- In the trace, a line per batch with its overlap and stall.

### 11. Claims: a plan, and launches taken by whoever reaches them

Placement (§9) decides once, at `begin`, from estimates. Under a load spike the estimate is wrong: the cores'
pool is taken by other work, or the GPU's worker is still busy with an earlier batch. A launch placed there then
waits, while another lane sits idle.

So placement makes a **plan**, and the lanes **claim** launches as they reach them (`exec::Claims`):

- Every launch has its planned lane, and the time by which that lane should have started it.
- Every lane that can run a launch has it on its list. The cores' list runs largest first. A device takes all of
  its planned launches in one submission.
- A lane takes its own launches first. It takes another lane's only while that launch is still free, and only when
  that lane is **behind its plan** by more than the taker's own time for the launch (at least 0.2 ms).
- A claim is one atomic exchange, so no launch runs twice. A pinned launch stays on its lane.

Who takes what:

- **The cores' pool** takes its own launches, then any launch of a device whose worker has not reached the batch
  in time.
- **A device** takes, after each submission of its own, the cores' launches their pool has not reached. A device
  is sent a batch only when it has launches planned in it.
- **The waiting caller** runs free launches itself, on the cores, instead of stalling. It takes the cores' own
  ones, and those of a lane behind its plan.

With no spike the plan stands: a device starts a batch within microseconds, so its launches are never late, and
nothing moves. The claims only move work when a lane falls behind.

**Chaining levels.** A level's batch also carries the next level's seed joins whose relations are already held.
Their launch, and the device's 1.7 ms of submit and schedule, then run under this level's strata instead of at the
start of the next one. The next level's batch skips a join already sent. A join sent early is taken by its spec's
digest and its relations' identity, as any other (§5). If either changed, the stratum runs without the pairs.

**What the explain output adds:**

- Per kind of batch: launches as planned and as run, per lane; launches moved off their planned lane; launches
  the caller ran while it waited.
- In the trace, the same per batch.
- The level joins' decision line says how many of a batch's joins were the next level's.

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
   (`rows_sort`) and their measured prices. *Done*:
   - `Kernel::Join` takes both sides' key columns. It returns the pairs in left row order, then right row order.
     `by` picks the device's way: sort and probe, or document tiles. The pairs are the same either way.
   - The CPU reference is `rerail_accel::rows::probe`: a radix sort of the right and a binary search per left row.
     The host sort moved from the engine into `rerail_accel::rows`. The device's sort, probe and tile moved into
     `rerail_accel::gpu::join` and `gpu::tile`. The engine's modules re-export them for the lab and the row
     programs.
   - Prices, per row of both sides: the cores 5.7 ns (one core); the GPU 1.4 ns by sort and probe, 5 ns by tiles
     (the host's grouping). The pairs are not known ahead, so they are not priced. At these prices a single join
     goes to the cores, as the lab found (0095).
   - **The space.** Every lane keeps a join's right side by the caller's id: the cores its sort; the GPU its sort,
     or its grouping by document. The id names a relation and its key columns, and a kept entry whose row count
     differs is refused. `Executor::forget(id)` drops one entry and `Executor::clear()` drops all of them, at the
     chunk's end. On the GPU, the bytes kept are capped at 256 MiB, and the least recently read entries go first.
   - **A batch's joins on the GPU** take at most two command buffers. The first holds the sorts the space lacks,
     every probe and every tile count. The second, only when needed, holds the tile scatters and the probes whose
     pairs outgrew their first space.
   - A test runs probes and tiles on both lanes in two batches, and checks them against the reference. The second
     batch reads the first one's kept sides.
5. **Levels in the engine:** a level's launches as one batch. Then first joins from the device, with the host
   continuing from the pairs. *Built*:
   - When a level's first stratum runs, the planner plans the seed rules of every stratum in the level. These are
     the rules with no recursive atom, and the planner keeps their plans for the strata. Their join specs go to
     the set engine, which sends the ones that qualify as one batch (`setfix::level`).
   - A seed qualifies when its first two steps are positive atoms of relations held by earlier levels, and the
     second is keyed by slots the first binds. Its left rows are the first atom's live rows that pass its tests.
     The right side is every row of the second atom's relation, named in the space by the relation's id and its
     key columns.
   - When the seed's stratum runs, the seed takes its pairs by its spec's digest, if both relations are the ones
     the batch read (same id, rows and dead rows). It continues from its third step. Each pair's right row is
     checked as the host's walk checks it: live, every key, every test.
   - Otherwise, the seed runs as before. Pairs that no seed took are dropped at the chunk's end, with the lanes'
     space.
   - `RERAIL_LEVEL_JOINS=off` turns the batch off. Joins with fewer than 4,096 left rows are left to the host
     (`RERAIL_LEVEL_JOIN_MIN`).
   - On the Python pack, no seed join reaches 4,096 left rows, so the default sends nothing. The findings are
     identical either way. With the floor at 0, 153 joins go out over 22 chunks and 132 seeds run from their pairs.
     The findings are still identical, and the relational lab is exact on the moved kernels (232 of 232 probes, 110
     of 110 tiles). Whether a lower floor pays is measured on the data layer's 9,010 files (step 7).
6. **The cores:** measure strata of one level in parallel, and decide. *Measured*: `RERAIL_LEVEL_TRACE=<file>`
   writes each native stratum's level and thread CPU. On the Python pack (22 chunks, 11,942 native strata in 401
   levels, load 26–67):
   - The native strata take 11.3 s of thread CPU, out of 31.5 s user + sys for the run.
   - A level's slowest stratum, summed over the levels, is 2.8 s. So running a level's strata at once could cut
     wall time by at most 8.5 s, 4.1 times on that part.
   - Half the levels have 8 strata or more, and 113 have 20 or more. The median stratum takes 0.11 ms, and the
     slowest takes 157 ms.

   **Decided: not now.** Parallel strata save no CPU. User + sys can only grow, through the symbol table's
   write lock (new symbols), the store's locks and the threads' handoffs. CPU is what the gates count, and the
   machine is shared, so its cores are busy anyway. The wall-time gain is real but bounded by 8.5 s here. It is
   worth taking when wall time is the target (a single user's run on an idle machine). The trace gives the bound
   for any corpus. The work queued for that case: strata of one level that read no SQL, on the rayon pool, each
   with its own interner overlay merged at the level's end.
7. **Every lane at once, every job scored (§8).** *Built*:
   - The tests run four batches from four threads, each on every lane at once and again on the cores. Every
     output is the reference's, the scoreboard counts every launch, and up to 5 jobs ran on the cores at once (4
     threads, one job run inside another's parallel loop). Host jobs that wait for host jobs, three times as many as
     the threads, all finish, and a panic is an error.
   - The pack's findings are identical on both bindings. With the level batch's floor at 0, the scoreboard shows
     176 launches over 22 chunks: 153 joins, 10 count tables, 9 histograms, 4 closures. Up to 4 ran at once.
   - Every one of them is placed on the cores, under the Metal binding too. The largest join is 1,057 rows, a few
     µs of work, against the GPU's 0.2 ms a command buffer. The device lanes take work when it is larger: the
     data layer's 9,010 files are the test (step 8).
   - Their jobs waited 0.21 s in the queue in all, against 0.08 s run, at a machine load of 80–190 (threads woken
     late under that load). User + sys is unchanged: 44.1 + 2.5 s on the default run, against 44.2 + 3.4 s before.
   - The run's peak resident memory is 1.45–1.50 GB, over the 1 GB budget. It was 1.44 GB before this step.
   - **Host CPU by phase.** Each job's thread CPU, and each phase's CPU and wall time, are on the scoreboard and in
     the profile's executor section. The GPU lane also records each command buffer's timeline (submit,
     schedule, run, wake). `RERAIL_LANE_PIN=<lane>` places every launch a lane can run on it, to measure.
   - The pack under `--metal`, the floor at 0, each lane pinned in turn (load 59–102):

     | | GPU lane | The cores |
     |---|---|---|
     | Jobs (launches) | 101 (176) | 176 (176) |
     | Thread CPU, all jobs | 28 ms | 51 ms |
     | Warm job (after the first 20) | 204 µs | median 53 µs, p90 1.25 ms (joins) |

     The GPU's warm job splits as:
     - encoding, 95 µs (about 5.5 encoder calls, 17 µs each);
     - the uploads' narrowing copy, 32 µs;
     - commit with the budget check, 16 µs;
     - reads, 13 µs;
     - the wait, 8 µs;
     - about 40 µs not in a phase.

     A command buffer's wall time under that load is 3.3 ms (submit 0.7, schedule 1.0, run 0.4, wake 1.1),
     with no host CPU in it. The first jobs compile their pipelines: up to 165 ms of wall time, but 2.8 ms of
     this thread's CPU (Metal compiles in its own service). Metal's own threads are not in the job's CPU.
   - **The run's host CPU is elsewhere.** The native set engine's join walks take 15.1 s, in 10,586 plans. The 11
     plans over 0.1 s each take 4 s. The rest average about 1 ms a plan. The scalar summaries take 2.3 s. The
     statistics and plans take 3.1 s and SQL 0.35 s. The executor's launches take 0.05 s, and building the level
     batch's key columns 0.03 s.
8. **Placement by host CPU (§9).** *Built*:
   - A unit test shows the rule's three cases. Within the slack, the lane of least host CPU wins though it
     finishes later. With no slack, a lane far slower is not a candidate. Load stretches the cores' wall time.
   - The pack under `--metal`, the floor at 0, findings identical in every run (load 70–130):

     | | GPU pinned | Cores pinned | Unpinned |
     |---|---|---|---|
     | Level joins on the GPU | 153 | 0 | 28 |
     | Host CPU of the launches | 25 ms | 47 ms | 27 ms |
     | Filling the columns | 3.3 ms | 3.2 ms | 2.9 ms |
     | Callers' stall, level joins | 370 ms | 82 ms | 169 ms |

   - Unpinned, 26 of the GPU's 28 joins were not on their fastest lane: the slack let the lane of less host CPU
     take them. The estimate was 17 ms of host CPU against 39 ms on the fastest lanes.
   - The cores' actual host CPU is 6 to 15 times the price's estimate at this load. The prices were measured on an
     idle machine. The measured ratio now corrects them.
9. **GPU jobs of a few µs (§9).** *Built*:
   - Tests cover small joins with one to three keys, negative and repeated keys, empty sides and staged columns,
     on both lanes, against the reference.
   - A warm loop of one small join a batch (`warm_small_gpu_jobs`, load 65–135):
     - **33.3 µs** of host CPU a job, or **32.6 µs** staged. It was about 200 µs.
     - Encoding takes 12.5 µs, commit 5 µs, the wait 4 µs and the reads 1.7 µs.
   - In the pack's run a job costs more: its jobs carry several launches, and thread CPU swells under a load of
     100 and more.
10. **Commit first, fill after (§10).** *Built*:
    - In the warm loop, 46 of 10,050 batches reached their commit before the host had filled them. The device
      waited on the event for those; the rest were filled while the lane encoded.
    - In the pack, the fill takes 42 µs a batch on the caller's thread, while the lane encodes and commits.
11. **The open GPU items (§9).** *Built*:
    - **Pipelines prewarmed.** The prewarm thread now also compiles the radix sort of each width, the gathers and
      the pair sort. A variant's source is compiled once, into one library, for all its kernels (a sort's five
      kernels were five compiles). A job that needs a pipeline still compiling waits for it (`pipeline wait`)
      instead of compiling it again.
    - **Spilled probes** run again through the fused probe, all in one dispatch, into one arena of their exact
      total. The separate one-probe kernel, whose many width variants were compiled on first use, is gone.
    - **Pairs as views.** A probe's pairs are handed out as segments of the arena (`Output::View`, `Pairs`),
      which the view keeps. The level batch hands each seed its pairs once, without the copy it made before.
    - **Tile pairs sorted on the GPU,** in the scatter's command buffer: a key of (left row, right row) in 32
      or 64 bits, the radix sort's passes alone, and a gather (`encode_sorted_pairs`, with a test of both key
      widths).
    - Results, the pack under `--metal`, findings identical (load 30–110):

      | | Before | After |
      |---|---|---|
      | Unpinned: callers' stall, level joins | 169 ms | 28 ms |
      | Unpinned: longest stall | 62 ms | 5.7 ms |
      | GPU pinned: pipeline compiles in jobs | 12 | 0 |
      | GPU pinned: host CPU of the joins | 25 ms | 13.4 ms |
      | GPU pinned: callers' stall | 370 ms | 101 ms |

    - Warm loop at a load of about 107: 50–53 µs a job, against 54–55 µs for the build before, back to back.
      Reading the pairs went from 2.4 to 1.9 µs a job. The rise from 33 µs is the load.
12. **Claims and chaining (§11).** *Built*:
    - A test makes the GPU lane's worker busy for 300 ms and plans a batch's 12 launches on it. The caller runs
      all 12 on the cores while it waits; the batch stalls 2 ms.
    - In the pack, unpinned, 1 of 153 launches moved. Pinned to the cores, the waiting caller ran 38 of 153 itself,
      and the callers' stall was 16 ms (82 ms before, with the pool alone).
    - Chaining puts up to 13 of the next level's joins in a level's batch. The run sends 153 launches and runs 132
      seeds from their pairs, the same as without chaining.
13. **Gates:**
   - The pack's findings are identical, and so are the data layer's at scale.
   - Host CPU (user + sys) of the device joins is below the CPU radix sort join's, on the lab's 232 joins and on
     the pack.
   - Submissions per chunk are at most one per lane per level, and the profile shows the space's hits.

## Open

- **A device that had nothing planned** in a batch is not sent it, so it cannot take the cores' late launches
  there; the waiting caller does instead. Sending it would hold its worker on a batch it may never serve.
- **Late is measured against the plan's own model** of the cores (one queue of their estimates), so they are
  rarely late. Whether to model the pool's threads is open.
- **Pipelining the GPU's submissions.** The GPU's worker waits for each command buffer before it takes the next
  batch. Chaining gives a batch more of the next level's joins instead.

- **Count tables and the scalar summaries overlap.** The engine's per-key memo of scalar summaries answers a count
  once per key, and a count table answers it once per relation. Since the merge, a count the memo serves never walks
  enough to build a table, so the count-table test is ignored. Which of the two a level's batch should send, and
  whether the memo should read the table, is decided with step 5.

## Not decided here

- **The Neural Engine in levels.** It is a lane like the others, so a level's bit products reach it through the
  same batch. Whether any of the engine's strata are worth it is 0098's to measure.
- **Recursive strata on the device** stay on the work queue (0027) and are not batched across strata.
