# 0113: Strata issued from a scoreboard: merged into components first, then run in parallel on the one executor

Status: draft (2026-11-11); steps 1–5 built in a first form (2026-11-11, `logic::plan::issue`, `accel::stream`),
step 6 in part. What is built, measured, and where it departs from the text below is in **Built so far** at the
end. It builds on 0100 (one executor, levels as
batches, claims, one pool) and takes up two items 0100 and 0101 left open: 0100 step 6's queued shape for parallel
strata, and 0101 §3's merged component.

## Intro

**The problem.** A run executes its program one stratum at a time. A stratum is a group of relations that
depend on each other. For each one, the strata loop in `logic::plan::run` does the same fixed work, whatever
the stratum's size:

- it lends the run's state;
- it gathers the stratum's statistics and plans its rules;
- it builds the join spec and places it;
- it opens the fixpoint;
- it drives the stratum, writes its outputs back, and records its choices.

On a 1-file run, nearly all strata are tiny, so the fixed work is most of the cost. Measured on the profiling
build: 278 programs, 16,644 rules, one Python file, 10 findings, 1 ms sampling.

| | Local run (per chunk) | Cross-file session |
|---|---|---|
| Strata | 9,985 | 10,604 |
| Levels | 44 | 67 |
| Strata under 0.2 ms | 7,798 (78%) | 10,099 (95%) |
| Strata of one rule | 8,141 (82%) | 8,414 (79%) |
| Median stratum (thread CPU) | 69 µs | 21 µs |
| Widest level | 1,703 strata | 947 strata |
| Thread CPU, all strata | 1.97 s | 1.10 s |
| The levels' slowest strata, summed | 0.10 s | 0.45 s |

Executing the strata takes 3.28 s of the run's 21.9 s of busy CPU. The joins and closures themselves
(`setfix::run`) are 0.48 s of that. The other 2.8 s is the fixed work:

| Per-stratum step | CPU (1-file run) |
|---|---|
| Planning the stratum's rules (`plan_search`, `plan_rule_reused`) | 0.88 s |
| Choosing what to keep (`keeps::choose`, priced rows) | 0.39 s |
| The SQL kernel's component (`sql_kernel::component`) | 0.39 s |
| Recording plan choices (`choices::write`) | 0.29 s |
| Splitting negations, lowering, SQL names | 0.58 s |
| Lending state, statistics, specs, placement, open | the rest |

So two different gains are on offer, and they are not the same gain:

- **Doing the fixed work less often** cuts CPU. The floor is one fixed cost per *merged component* instead of
  one per stratum.
- **Running independent strata at once** cuts wall time. It can't cut CPU. Running them at once costs threads,
  handoffs and locks (0100 step 6, measured), and a stratum's fixed work is paid on whichever thread runs it.
  The bound here is 20.6× on the local run (1.97 s over 0.10 s) and 2.5× on the cross-file session, where one
  stratum takes 179 ms.

The gates count CPU, and the machine is shared, so merging comes first. Parallelism comes second, and only
behind a gate that shows CPU did not rise.

**What changes.**

- **A scoreboard, not level barriers.** Each stratum counts the strata it reads that have not yet retired, and
  it can issue the moment that count reaches zero. A level becomes a lower bound on when a stratum can run, not a
  barrier every stratum of the level waits at. This is the scoreboard of an out-of-order CPU, applied to the
  dependency graph `by_level` already computes.
- **Merged components.** A run of small strata in the scoreboard's ready order goes out as one component, with
  one plan cache entry, one open and one statistics pass. Its strata run inside it as fixed,
  ordered sub-steps, each to its own fixpoint. This is 0101 §3's "stratified sub-steps", built once for every
  stratum rather than only for the pieces of a DRed rewrite.
- **Strata are jobs on the one executor, and the executor decides the parallelism.** A component that is ready
  goes to `rerail_accel::Executor` as a host job with its estimate. It is scored, claimed and helped like every
  other job. How many components run at once, and on which threads, is the executor's decision, from its
  scoreboard, its job floor and the machine's load. The strata never read the thread count. There is no second
  scheduler (0100 §1), and one pool runs everything (0100 §12).
- **In-order retirement.** Components may finish out of order, but each publishes its outputs, new symbols and
  statistics in stratum order. This is the reorder buffer of an out-of-order CPU. The run's outputs, symbol
  ids and stored state are then the same as a serial run's.

**A concrete case.** The local run's widest level has 1,703 strata, most of them one rule over `node` and a
`kind` test. Today that is 1,703 plans, 1,703 opens and 1,703 statistics passes, one after another. Under this
decision, the scoreboard issues them as soon as their inputs (the chunk's base relations) are held. They go out
as about 20 components of about 85 strata each, priced so that each costs well over its fixed work. Those
components then run on as many threads as the executor gives them, and retire in order.

## Prior work consulted

- **ast-index-spike design index, partitioning (D S145).** "Partitioning alone is not a win (it pays the fixed
  cost P times)." Client parallelism over 8 connections gave up to 10.5×.
  - **Agrees.** Splitting the strata over threads without merging them pays each fixed cost on another thread,
    and the CPU is unchanged or worse. So merging comes first, and parallelism takes components, never strata.
- **Design index, parallel indexing (S182).** 4.1× on 4 workers and 7.3× on 8, but the merge of shards was
  never resolved (the vocabulary buckets were contended).
  - **Agrees and departs.** The shard merge is this design's interner overlay. It is resolved here by
    retiring in order, so only the retiring thread writes the shared table and the buckets have one writer.
- **History index D61.** Parallel append in the planning round was 2× worse.
  - **Agrees.** Parallelism that adds coordination per small unit loses. Here the unit is a priced component,
    not a stratum.
- **Lessons ledger row 1.** One task object per tuple is fatal.
  - **Agrees.** One job per stratum would be the same mistake one level up: a median stratum of 21–69 µs
    against a job's handoff of a few µs. A job is one component.
- **Ledger row 16.** Fusing everything into one statement does not remove repeated work. Fix plans and candidate
  sets first.
  - **Agrees.** A merged component shares fixed work only: the open, the statistics pass and the plan cache
    entry. It does not fuse its strata's evaluation. Each sub-step keeps its own rules, its own
    plan and its own fixpoint.
- **Ledger row 26.** Acceptance is structural: work is proportional to the inputs and the results.
  - **Agrees.** 20,589 fixed costs for a 1-file run is work proportional to the program, not to the file. The
    gate counts components opened per run and fixed CPU per component.
- **Decisions.**
  - 0010 (SQL frontier waves). Unchanged. A stratum that runs in SQL issues from the same scoreboard, but always
    on the run's thread, because the connection is not `Sync`.
  - 0027 (the device work queue). **Used** unchanged for recursion. A recursive stratum is one sub-step and
    never splits across components.
  - 0090 (levels strata) and 0077 (DRed). A levels stratum, or a maintained one (`__del`, `__ins`, `__old`),
    is a sub-step like any other. Its driver's rows are written inside the component.
  - 0095 §5 and 0100 §5 (a level is one batch). **Generalized.** The batch is "the components that issued
    together". Count tables, seed joins and chained joins are collected from the components issued, not from a
    level.
  - 0100 §1 (one executor) and §12 (one pool; a job that waits for a long job is not itself a job). **Kept.**
    A component waits only for device launches, through the existing suspend and resume (`Native::Waiting`,
    `finish_wave`). It never waits for another component, because the scoreboard only issues components whose
    inputs have retired.
  - 0100 step 6 (parallel strata: "not now"). **Revisited with the reason it gave.** That step measured strata
    run in parallel unmerged, and found no CPU gain. Here merging cuts CPU, and parallelism is a separate gate.
  - 0101 §3 (a stratum's rewritten pieces as one component). **Subsumed.** The DRed pieces of one stratum are
    the first case of a merged component.

## Decision (proposed)

### 1. The scoreboard

The strata loop keeps its order (`by_level`, dependencies first) as the program order. Beside it, a scoreboard
keeps per stratum:

- **pending**: the number of strata it reads, by `deps` (reads, answered requests, a native operator's inputs),
  that have not retired;
- **readers**: the strata that read it, whose `pending` drops by one when it retires;
- **state**: waiting, ready, issued (in a component), suspended (on a device wave), or retired.

At the start, every stratum whose `pending` is zero is ready. Retiring a stratum makes its readers ready as
their counts reach zero. The scoreboard is a plain array indexed by stratum: no hashing of names, no locks
(only the retiring thread touches it, see §4).

This replaces the level barrier, not the level. `level_of` stays, both for the trace and for chaining, and the
first ready set is level 0. A stratum of level 5 whose inputs all retired early issues early. Today it waits
for all of level 4.

### 2. Merged components

The issuer packs ready strata into components by their cost alone. It never asks how many threads are free:
that is the executor's decision (§3). A component is the grain of work, sized so its fixed cost is small
against it:

- **Members.** Ready strata, taken in program order. A component may also take a stratum that becomes ready
  only through its own earlier members: a chain `a → b → c` of single readers is one component, run as
  sub-steps `a`, `b`, `c`.
- **Size.** Members are added until the component's estimate, the sum of its members' prices from the cost
  model (0100 §14), is at least *k* times one fixed cost. *k* starts at 8 and is fitted, as the cores' job
  floor is. A stratum whose own estimate is over the bound is a component alone.
- **What it shares.** One `lent` of the run's state, one statistics pass over the relations its members read,
  one plan cache entry keyed by the members' rule digests in order, one open of the fixpoint space, and one
  record of plan choices, kept in memory (the store's writer keeps it later, 0102). A native stratum writes
  nothing to SQL, so a component has no write-back of its own.
- **What it does not share.** Each member keeps its own rules, plan, delta and fixpoint. A member's
  sub-step starts only when the previous one has reached its fixpoint. The meaning is the serial program's,
  by construction.
- **What stays alone.** A stratum that runs in SQL, one with Python natives, one that suspends on its device
  wave (a member that suspends ends its component there, and the rest go back to ready), and a recursive
  stratum over the cost bound.

The plan cache entry is what makes merging pay on warm runs too. Today `plan_rule_reused` looks up and rebuilds
per stratum. A component's entry is one lookup for all its members.

### 3. Issue: components as jobs on the one executor, which decides the parallelism

A component that is ready is submitted to the executor as a host job with its estimate. The strata decide
**what** may run (the scoreboard: what is ready) and **how big** a job is (the component). The executor decides
**how many run at once, and where**. No code in the strata reads the thread count, the pool's load or another
job's state.

Today `Executor::spawn` starts every job on the pool at once. That gives no decision to the executor, so this
decision adds one: a **host stream**, an open batch the issuer adds jobs to as they become ready.

- **The same claims as a batch's launches** (0100 §11). Each submitted job is a launch on the cores, scored on
  the scoreboard with its estimate, claimed by one atomic exchange. Pool workers claim them largest first. The
  issuer, when it waits for a component to retire, claims and runs free ones itself (`help_until`), as it
  already runs a batch's free launches.
- **How many pool workers serve the stream is the executor's.** It starts a worker for the stream only while
  the stream's outstanding estimate, spread over the workers serving it, is above its measured job floor (the
  cost below which a pool job loses to running inline), and while the cores' scoreboard and the machine's load
  (`cores_slowdown`) leave a thread. Otherwise the jobs wait for the issuer to claim them. With one thread
  (`RERAIL_THREADS=1`), or on a busy machine, every component runs inline on the issuer, in program order, with
  no other change.
- **One floor and one model for every host job.** The floor and the estimates come from the cost model (0100
  §14), fitted from the scoreboard's samples for the stream's form. The strata pass only an estimate per job.
- **The run's thread issues and retires.** It owns the scoreboard and the SQL connection. It runs SQL strata
  itself, in their program order, and helps with components while it waits.
- **Device launches still go out as one batch.** The count tables, seed joins and chained joins of every
  component issued at once go out in one submission per lane, as a level's do today (0100 §5, §11). A
  component that reads a launch waits on it inside its job, through the same suspend and resume as now.

### 4. A component's state, and retiring in order

A component must run off the run's thread, but today a stratum reaches state that is global or tied to one
thread:

| State | Today | In a component |
|---|---|---|
| The SQL connection (`Run`'s `&Connection`) | not `Sync` | not reached: SQL strata stay on the run's thread |
| Plans (`PLANS`, `SESSION_PLANS`) | thread-local | read shared; the component's new entries go in its own map, published at retire |
| Device placement (`placement::DEVICE`), the wave state (`lanes::wave`) | thread-local | per component, passed in |
| Profile spans | thread-local | per component, merged at retire |
| The interner (`setfix.rs`, a global `RwLock`) | shared, write-locked for a new symbol | read shared, with an overlay of the component's new symbols |
| `DERIVED`, `INPUTS`, `STORED` | global maps under a mutex | read shared; the component's writes go in its own map |

Retiring happens on the run's thread, in program order. A component that finishes early waits in a buffer until
every component before it has retired. Retiring a component:

1. merges its interner overlay into the shared table, renumbering its new symbols in order of first use. Its
   rows already hold overlay ids, so a renumbering map is applied once, to its own outputs only;
2. publishes its relations, statistics, plan entries, choices and profile spans;
3. retires each member stratum on the scoreboard, in order, which makes their readers ready.

Because retirement is in program order, and the overlay is merged in order, the run's symbol ids, its stored
facts and its choices match a serial run's. This is why this design needs no "shard merge" of the shared
tables: only one thread ever writes them.

**What can conflict.** Two components that are ready at once never read each other's outputs, since neither
is ready until what it reads has retired. They can create the same new symbol, and the in-order merge maps
both to one id. Nothing else they touch is written by more than one component.

### 5. What the explain output and the gates add

- **Explain.** Per run: strata, components, the members per component (median and largest), fixed CPU per
  component, components that ran in parallel (most at once), retire-buffer waits, and the critical path against
  the sum.
- **`RERAIL_LEVEL_TRACE`** gains the component of each stratum and the thread that ran it.
- **`RERAIL_STRATA_MERGE=off`** runs one component per stratum, and **`RERAIL_STRATA_PARALLEL=off`** runs
  components inline. Each gate below is measured against both being off.
- **The gates.**
  - **Merging.** On a 1-file run, components opened fall from about 20,600 to under 2,000. CPU for executing
    the strata falls by at least the fixed work saved: 2.8 s of fixed work over 20,589 strata now; target
    under 0.5 s. The findings are the same.
  - **Parallelism.** User + sys does not rise by more than the noise (instructions retired within 1%), and
    wall time falls on the 300-file benchmark. If CPU rises, parallelism stays off by default, as 0100 step 6
    decided, and the bound is reported.

## Order of work

Each step is measured before the next. Measure on the profiling build, report instructions retired, CPU and
wall, and check the 1-file and 10-file findings against the serial run.

1. **The scoreboard, serial.** Replace the level barrier with the scoreboard on the run's thread, issuing one
   stratum at a time in ready order. Same outputs, same CPU. The test: a program whose level-5 stratum reads only
   level 0 issues before level 4 ends.
2. **Merged components, serial.** Pack ready strata into components, and give them one plan cache entry, one
   open and one statistics pass. Run them inline. This is the CPU step. The gate: fixed CPU per run, and
   components opened.
3. **A component's own state.** Move the thread-local and global state of §4 into a per-component context,
   still run inline. Same outputs, same symbol ids. The test: a run with merging on and off gives identical
   stores.
4. **The host stream.** Add it to the executor (claims, the scoreboard, the floor, the workers it starts), with
   its own test: jobs below the floor run on the caller, and a loaded machine starts fewer workers.
5. **Components as jobs.** Submit them to the host stream, retire in order. The gate: instructions within 1%,
   then wall on 10, 50, 100 and 300 files.
6. **Device batches from the issued set.** Collect count tables, seed joins and chained joins from the
   components issued together, not from a level.

## Open

- **The merge bound *k*.** 8× the fixed cost is a prior. It is fitted from the scoreboard's samples, with the
  number of members as a feature, as the job floor is.
- **A plan cache entry per component on warm runs.** The key is the members' rule digests in order. Whether a
  component's membership is stable across runs (the same ready sets, so the same components) decides whether
  the key should be per stratum, with a component as a list of keys.
- **The renumbering cost.** Rows that hold a new symbol are rewritten once at retire. On a 1-file run few
  symbols are new after the program's constants. On a cold 300-file run this cost is measured in step 3.
- **The cross-file session's long stratum** (179 ms) bounds that run's parallelism at 2.5×. Whether it should
  run in parallel within itself (by file, or by key) is not decided here.

## Not decided here

- **Strata on the device.** A component is a host job. Its device launches go through the executor as now.
  Moving a whole component to the GPU is 0027's and 0100's to decide.
- **SQL strata in parallel.** They stay on the run's thread, with its connection. A connection per thread is a
  separate design.

## Built so far (2026-11-11)

**What runs.** The strata loop is the retire loop. Beside it, `plan::issue::Board`:

- keeps the scoreboard of §1, as pending counts and readers by stratum;
- at each stratum the loop reaches, packs the ready strata from there on into components (§2): ready strata in
  program order plus chains through the component's own members, up to *k* = 8 fixed costs or 64 members;
- sends the components' count tables and seed joins as one device batch (step 6, for the issued set);
- submits each component to the executor's host stream (§3) with its estimate.

A component retires at its first member's position. Its members run in order on one strand: a `Run` over a fork
of the run's model (`Model::fork`), holding the statistics of the relations it reads, the run's scope, and no
connection. Members that wait on their device wave share one wave.

**The host stream** (`accel::stream`) claims largest first and scores each job on the executor's scoreboard. It
starts workers only while the queued estimate per worker is above the measured floor and the cores leave a
thread, at most one per queued job, and never again for a worker that ended while it was starting them. (It once
did: a worker that found nothing to claim ended at once, and the submit started another, until every thread the
cores leave happened to serve at once. On the data-layer replay the issuing thread spent most of its time there,
and the pool's threads spun on the workers it started.) The
waiter runs its own job if no one claimed it. A job never runs inside another on one thread.

**Switches.** `RERAIL_STRANDS=off` gives the serial loop. `RERAIL_STRATA_PARALLEL=off` runs components at
their retire. `RERAIL_STRATA_MERGE=off` gives one stratum per component.

**Measured.** O125 replay, steps 1–8, profiling build, load average 21–47 (wall time is not comparable):

| | same as full | instructions retired | user + sys |
|---|---|---|---|
| strands off | 8 of 8 | 576.6 G | 35.8 s |
| merged, run at retire | 8 of 8 | 620.0 G | 41.1 s |
| merged, parallel | 8 of 8 | 579.5 G (+0.5%) | 43.5 s |

Components: 6,628 for 10,700 strata issued, of which 43 ran on the loop's thread after all. The parallel gate
on instructions (within 1%) holds. The merging gate does not: about 1.6 strata per component, and no fixed work
is shared yet beyond the fork and the scope's prime.

**Departures, and what is left.**

- **The interner overlay (§4) is not built.** A component gives out no new atom (`atoms::held_only`). A member
  that would make one stops, and runs in order on the run's thread. So atom ids are the serial run's, and no
  commit meets another thread's pending atoms (0049). The cost is that strata which make strings run serially.
- **Sessions are not issued:** a run with kept strata, strata derived again, or deferred deltas runs serially.
  Nor are answered requests, native operators, lattices, facts, `Ast` literals, or the relational lab.
- **Shared work per component** (one plan cache entry, one open of the fixpoint space) is not built. Each member
  still opens and plans alone. That and the low merge ratio are step 2's CPU gain, still open.
- **Device batches go per component** for waves (members share one). Count tables and seed joins go per issued
  set.
- **Estimates** are the fixed cost plus 20 ns per row read, not the cost model's prices (0100 §14).
- **Not yet measured:** wall time at 10, 50, 100 and 300 files on a quiet machine, and the 1-file and 10-file
  findings.

