# 0114: Suspend, never block: every wait is a scoreboard event, and answers are loaded as batches on workers

Status: draft (2026-11-11). §1 and §5 are built (`plan::issue`, `content_store::atoms`), and §3's posting and
resuming (see **Built so far** at the end); the rest is proposed.
It builds on 0113 (strata issued from a scoreboard) and 0100 (one executor, claims, one pool), and takes up
the requests answered inside the fixpoint of 0065 §2a and the load order of 0108.

## Intro

**The problem.** 0113 made independent strata run at once, as jobs on the executor's host stream. But the run
still has threads that wait by blocking: they sit in a loop, or help the pool, until something else is done.
A blocked thread holds its stack, its strand and its place in the order, and nothing behind it can move.
Measured on the O125 replay (steps 1–8, profiling build), these are the waits:

| Wait | Where | What it waits for |
|---|---|---|
| An answer | the cone's `answer` hook (`plan/corpus.rs`), `help_until_as` | the main thread to load documents and run them; 27 calls, 0.67 s |
| A launch | `exec::Each::take`, inside a component's job | its own device or CPU launch of a level batch |
| A wave | `Strand::run_members` → `lanes::wave::flush` | the device wave its members share |
| A component | `Board::retire_at`, `Stream::wait` | the job that holds the stratum at the loop's position (it polls) |
| Tables ahead | `prefetch_tables`, on the issuing thread | count tables, before the issued components are submitted |

The first is the largest. An answered request makes its stratum run on the loop's thread, and the loop
can do nothing else until the answer comes back. With the loop's waits counted, the trace gives:

| | serial | parallel (0113, readiness by completion) |
|---|---|---|
| Span of the strata loop | 5.84 s | 2.48–2.61 s |
| Critical path (the longest chain of strata by what they read) | 2.23 s | 1.97–2.07 s |
| On the loop's thread for "an answer produced" | — | 0.68–1.0 s |

So the span is within 1.26× of the critical path, and the answers are most of what is left over.

**Why an answer is on the main thread today.** A load request goes through four steps:

1. **Asked one at a time.** The cone sends one request and waits for its reply. Nothing gathers the requests
   of several strata, or of one level, into one batch.
2. **Parsed on workers.** The main thread sends the documents to the chunk builder, whose jobs parse them or
   take them from the memo. This step is parallel already.
3. **Evaluated on the main thread.** Each built chunk runs its file-local program on the main connection
   (`run_atoms_native_over(.., &db, "main", ..)`), with state that can't be sent to another thread: the
   exports, the statistics keeper, the outputs, the base rows (`RefCell`s of the corpus loop).
4. **Written back.** The chunk's export rows are added to the exports, and the answer rows (`rows_of`) go back
   to the cone.

Only step 4 needs one order. The exports must take rows in the order 0108 fixes (where the load budget binds:
by module depth, module, then path), so that a run is the same however its threads ran. Steps 1 to 3 need no
order at all.

**What changes.** No thread ever waits on another by blocking:

- a stratum that needs something it doesn't have **suspends**: its state goes back to the board;
- what it waited for **arrives as an event**: an answer, a launch's end, a job's end;
- the event **makes the stratum ready again**, and it resumes as a job, like any ready component;
- the loop's thread is the **retire step**. It sleeps only when nothing at all is ready, and it is woken by
  the next event, not by a poll.

Answers become jobs too. The requests of every suspended stratum are gathered into one batch, the batch's
documents are loaded and evaluated on workers, and only the write-back runs in one place, in 0108's order.

```text
  today                                     this decision

  stratum ─ask─▶ main ─parse─▶ workers      stratum ─suspend─▶ board (requests posted)
     │ (spins)     │ ◀─chunks─┘                  board: batch of requests ─▶ jobs (load + evaluate)
     │             │ evaluate (main db)          job ends ─event─▶ board
     │ ◀──reply────┘ write back                  retire step: write back in 0108 order
     ▼                                           stratum ready ─▶ resumes as a job
  goes on
```

## Prior work consulted

- **0113** gave the scoreboard (`plan::issue::Board`), the host stream, strands and their forks, and the
  switches. Its own suspend (a stratum waiting on its device wave, `Native::Waiting`, resumed by
  `native_resume`) is the pattern used here for every wait.
- **0100 §11** gave claims (an atomic exchange per launch) and the rule that the executor alone decides how
  many jobs run at once.
- **0065 §2a** defined requests: a component with a pending request pauses (`setfix` returns "2"), its
  requests are answered, the answers are added to its produced relations, and it goes on.
- **0108** fixed the order in which loads are taken when the load budget binds, and that a load asked again
  is answered from the exports, not sent again.
- **0049** made pending atoms a reason to refuse a commit; §5 here replaces that refusal with a reservation.

## Decision (proposed)

### 1. Readiness comes from completions (built)

A stratum is ready when what it reads has retired, not when the loop reaches it. `Board::collect` takes each
finished component as it ends, out of order, and drops its readers' pending counts. The loop issues from its
own position, inclusive, so a stratum the loop is at can join a component. Built in 19238d91b.

### 2. Waits are scoreboard states

A stratum, or a component, is in one of these states on the board:

| State | Means | Leaves it when |
|---|---|---|
| waiting | something it reads has not retired | its pending count reaches zero |
| ready | can run | it is issued |
| issued | in a component, as a job | the job ends |
| suspended on answers | its requests are posted | every one of its requests is answered |
| suspended on a launch | its launch is out | the launch ends (`Each`'s completion) |
| done | its rows are final | — |

A job that reaches a wait returns, with what it needs to go on: a continuation, like `Driving` today. The
board records what it waits for. When the event comes, the board puts the continuation back on the host stream
as a ready job. No job ever calls a blocking wait, a `help_until`, or a poll loop.

### 3. Answers: posted, gathered into batches, loaded on workers, written back in order

- **Posting.** When `setfix` returns "2", the driver reads the pending requests, as now, but doesn't call the
  hook. It returns `Native::Suspended { on: Requests(asked), d }`, like "3" for a device wave. The board
  records the requests against the stratum.
- **Batching.** At each issue point, the board takes every posted request not yet sent, from every suspended
  stratum, and makes one batch per kind (`load`, `dir_entry`, `file_digest`, `file_line`). A request already
  sent or answered isn't sent again; its answer is shared.
- **Loading.** A batch's documents go to the executor as jobs: parse or memo (as the chunk builder does now),
  then the file-local evaluation of each chunk on a connection of its own (a scope of the store, as a strand
  has), not the main one. Their outputs are the chunk's rows: exports, outputs, base rows, statistics. Each is
  an owned value, not a write to shared state.
- **Write-back.** The jobs' results come back as events. The retire step applies them in 0108's order: rows to
  the exports, lines to the outputs, statistics to the keeper. Then it gives each suspended stratum its
  answers (`component::add`, or `answered` for a produced relation outside the stratum). This is the only
  point that needs one order, and it is a short merge of owned values, not an evaluation.
- **Resuming.** A stratum whose requests are all answered becomes ready, and resumes as a job.

The cone keeps its own run and thread for now. What changes is that its loop is a retire loop like the
main run's: it posts and goes on with whatever else is ready. The main thread stops being the evaluator of
loads; it holds the corpus state the write-back goes into until that state is owned by the retire step (§7).

### 4. Launches and the loop's own waits

- **A launch.** A component whose member needs a launch's result (`Each::take`) suspends on that launch.
  `Each` reports each launch's end as an event (its completion today only wakes a waiter). The board resumes
  the component when it comes.
- **A wave.** Members that share a device wave suspend on it together. The wave goes out once (as now), and
  its end resumes them. `run_members` no longer calls `wave::flush`.
- **The loop.** When nothing is ready and nothing can retire, the loop's thread waits on the stream's
  completion event (one condition variable for every job end, answer and launch). It doesn't poll, and it
  doesn't run jobs itself unless the executor gives it one.
- **Tables ahead.** `prefetch_tables` becomes a job of the issued set. The components that read its tables
  wait on it as on any producer, by a pending count.

### 5. Atoms: reserved by run, published at retire (built)

Every run takes its own cursor over the interner (`Interner::for_loading`), not the shared write lock. Ids are
given in scheduling order, not the serial run's. That is sound because no output depends on an id's value, only
on what it stands for (0111).

The atoms a component makes are **reserved** for that component (`atoms::reserving(&arena)`), keyed by the
arena they were made in, not by thread. A reservation ends when the component's job ends (`Reserved`), and is
dropped when the component retires. `Atoms::pending()` does not count reserved atoms, so a commit elsewhere
isn't refused because of another component's atoms. The atoms become visible, and committable, with the rows
that name them. Built in 72db0aec6, with the test
`reserved_atoms_wait_for_their_component_then_commit_with_rows`.

### 6. What the profile shows

Every wait is named and timed in the explain profile (`profile::decision` and `work`), not only in the trace:

- **The critical path and the span**, per strata loop, and their ratio.
- **The loop's thread, by reason:** retiring, issuing, write-back, strata run there and why (`strand_reads_with`
  reasons), and sleeping with nothing ready.
- **Suspensions, by kind:** answers, launches, waves. For each kind: how many, how long from suspend to resume,
  and how much of that was on the critical path.
- **Loading:** requests posted, batches sent and their sizes, documents loaded, parse and evaluation time on
  workers, write-back time, and requests answered from what was already loaded.

`RERAIL_STRATA_TRACE` gains rows for suspensions and events, so `strata_trace.py` can put them on the critical
path.

### 7. Later

- **Sessions and strata derived again** are issued as components. Their dependencies are recorded in the
  scoreboard (a stratum derived again is a new producer of the relations it rewrites).
- **Estimates** come from the cost model's prices (0100 §14), or from the measured duration of the same
  component's last run, not a fixed cost plus 20 ns per row.
- **Merging.** Raise the ratio of strata per component (6,628 components for 10,700 strata now), and share one
  plan cache entry and one fixpoint open per component (0113 §2).
- **The corpus state** (exports, keeper, outputs) is owned by the retire step, so the main thread is only the
  caller of the run.

## Order of work

1. Profile entries for the waits that exist now (§6): answer hook time and calls, loop thread by reason,
   critical path and span. They are the before.
2. Answers (§3): suspend on requests, gather into batches, then load and evaluate on workers, then write back
   at retire.
3. Launches and waves (§4): `Each`'s ends as events, `run_members` without `flush`.
4. The loop's wait on one completion event; `prefetch_tables` as a job.

Each step is gated on the O125 replay giving the same as full (8 of 8), and on instructions retired within 1%
of the step before.

## Open

- Whether a load's file-local evaluation can run on a connection of its own as is. It binds module sources and
  reads the main scope's tables (`modules.bind(&db, ..)`). A scope of its own may need those bound again, which
  costs CPU.
- Whether the write-back's order must be 0108's for every load, or only where the load budget binds.
- How the statistics keeper merges a chunk's statistics out of order (it counts files as they come today).

## Built so far (2026-11-11)

**Diagnostics (§6).** Each cone reports where its answers' time went: requests, documents loaded, taking them,
waiting for chunks built on workers, evaluating the chunks on the answering thread, reading answer rows back, and
that thread's wait for the next request. It goes in the profile as a decision, and to stderr as `RERAIL_ANSWERS`
with `RERAIL_STRATA_COUNTS`. On the O125 replay (steps 1–8):

| | |
|---|---|
| Requests answered, documents loaded | 45, 1,273 |
| Answering, on the main thread | 0.80 s |
| of which waiting for chunks built on workers | 0.34 s |
| of which evaluating the chunks on the main thread | 0.42 s |
| The main thread idle, waiting for the cone's next request | 0.61 s |

**Posting and resuming (§3, first half).** Where the strata loop retires from the board, a component with
requests posts them (`Asking::post`, a ticket each) and suspends (`Native::Asking`). The loop goes on. Answers are
taken as they come (`Asking::arrived`), and each stratum whose tickets have all been answered resumes. A stratum
that reads one still asking waits on the answers' channel, not in `help_until`. While it waits it takes the
board's finished components. Relations that a stratum still asking reads are not released or unloaded until it
finishes, because its fixpoint reads them again. The cone's channels carry tickets (`corpus::Asks`). The rounds
(`cone_rounds`) and strata needed at once still answer in place. `RERAIL_ASK_ASYNC=off` answers in place
everywhere.

Measured: same as full 8 of 8. On this replay the trace shows the posting gains no wall time. In the cone's
program, every stratum after the request's (`ask_loaded_bff`) reads it, so the answer is on the critical path and
nothing else can run beside it (span 2.05 s against 2.10 s waited in place, within noise). What shortens it is
the second half of §3: evaluating the loaded chunks on workers (0.42 s now on the main thread) and taking the
batch's chunks as they are built.

**Not yet:** the second half of §3 (load and evaluate on workers, write back in order), §4, and the rounds'
answers posted.
