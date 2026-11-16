# 0263: A join's scan walks in morsels across the pool

Status: building (2026-11-15). Refines the native engine's join walk (`setfix::run_join_in_each`) and the kept
part's frames (`Retained::write_frames`, t33 K4). Follows decision 0254 (chunks evaluate at once), which left the
cross-file part serial.

## Intro

Once 0254 spread a run's chunks over the pool, the end of a run on all of PostHog was mostly one thread. The
cross-file part runs its strata as a chain, each one join walked on one thread. Before this decision, on an M3
Max at K = 15, the cross-file part ran from 35.5 s to 40.5 s of a 42 s run:

| what | time | how |
|---|---|---|
| `exports/resolve_export` | 1.0–1.15 s | one join over 443,866 delta rows (11 waves), one thread |
| `sym_of__sup0` | 1.0 s | one seed over 2.2 M `qn` rows, one thread |
| `sym_of` | 0.3 s | one seed, `@cat` per row |
| the kept part's frames (`write_frames`) | 0.8 s | every changed relation's rows digested and encoded in turn |

MLDB's answer to such a loop is `parallelMapChunked`: cut the range into chunks, give each chunk its own
accumulator, and merge the accumulators in chunk order, so the result does not depend on which thread ran which
chunk. Its joins (`joined_dataset.cc`) sort each side in parallel and then merge. That is the pattern taken here.

## Decision

### 1. A first step that scans many rows is walked in morsels

`run_join_in_each` asks `morsels` before it walks. A walk runs in morsels when:

- its first step is a positive atom with no key (a scan of an input, of a relation, or of its delta);
- the scan has at least 32,768 rows;
- there are at least two threads;
- nothing restricts a later step to a cut's rows (decision 0206).

Each morsel is at least 4,096 rows, and there are about four morsels per thread.

The scan's rows are put in the next step's key order once (`scan_order`, decision 0112), then cut into
contiguous slices. Each morsel walks its slice as a job on the pool (`a join's morsel`). It has its own:

- counts;
- interning cursor (the same arena);
- factored-step memory;
- row-program scratch;
- run hints;
- new rows (`Fresh`).

Before it ends, a morsel settles its rows: sorted by the relation's first order, deduplicated, and checked
against the relation. The walk then takes the morsels' rows in morsel order (`Fresh::absorb`). Each morsel's rows
are one stretch in order, so the walk's next `settle` merges the stretches rather than sorting them. A collecting
walk appends the morsels' rows in order, as one walk would read them.

The walk's counts are the morsels' counts summed, except step 0's entry, which is counted once per walk, as
before. The counts are kept in the walk (`Ctx::counts`) and added to the join's atomic `Count`s once, when the
walk ends. `JoinRule` is now `Sync`: a count step's table state (`StepForm`) is atomics and `OnceLock`s, and the
first morsel to pass the step's cost builds the table.

### 2. Under a budget, each morsel spends an equal share

A walk under a budget (decision 0206) gives each morsel an equal share of the work left. A morsel stops at its
share and reports the rows it left, from the row it stopped in. The walk's left rows are the union of those
(`Ctx::left0`), and the cut reports them as a set (`Rows0`). So:

- the walk's work stays within its budget;
- where it stops depends only on the rows, not on the threads' timing;
- the wave that goes on from the rows left reaches the same fixpoint.

`cut_tests::a_scan_walked_in_morsels_makes_one_walks_rows_and_cuts_within_its_budget` checks that:

- a 40,000-row scan walked in morsels makes the same rows and step counts as one walk;
- under a budget it cuts, its done and left rows add up to the delta;
- the cut's rerun reaches the same fixpoint.

### 3. Chunks evaluated at once walk whole

A chunk evaluated beside others (decision 0254) runs under `walks_whole`, which is carried into the chunk's
jobs (`run_state`: `WHOLE`, Carried). The chunks already keep the pool busy, so morsels there would only add jobs
and memory. On `posthog/` they raised peak memory from 1.39 to 1.64 GB. `RERAIL_MORSELS=off` turns morsels off
everywhere.

### 4. A join that reads texts row by row walks whole

Narrowed by decision 0264 §7: since the arena's reads take a shard of their own, only a join that may make atoms
walks whole.

This covers `@cat`, a split, a text function, a text test and a joined text. Each one reads the atom arena
under its lock (`Atoms::state`). A miss then goes on to `existing`, then `lookup` and `catch_up`, under further
locks.

On the M3, with 16 threads, `exports/resolve_export` walked in morsels took 2.7 s against 1.0 s on one thread,
for the same work units. A stack sample of that window showed every thread in `lock_shared_slow`, under
`Atoms::existing` and `Atoms::lookup`. Folding a `@cat` into one read of the arena (`Atoms::cat_held`) did not
change the 2.7 s, and neither did ending a miss without locks where it is final (`misses_are_final`). Both stay:
one walk does fewer lock reads with them too.

### 5. Reservations nest

While its morsels run, a walk's thread runs other jobs of the pool. Such a job can be another component, which
opens its own atom reservation (decision 0114 §4). That tripped `one reservation per thread at a time`, once in
four runs.

A reservation opened inside another now saves the outer one and restores it when it ends. Each component keeps
its own atoms. `atoms::tests::a_reservation_inside_another_holds_its_own_atoms` checks this.

Morsels make no atoms: §4 keeps every step that makes texts out of them.

### 6. The kept part's frames are digested and encoded on the pool

`write_frames` used to copy, digest (`rows_digest`) and encode (`cross_rows::encode`) each relation it writes
whole, one after another, on the main thread. That work is now done first, one job per relation: each job copies
the rows, digests them, and encodes them when they differ from the stored digest. The loop then makes the same
decisions in the same order, with each digest at hand, and names each frame by its reads
(`FramePut::encoded_as`).

A delta written after its frame stays in the loop (`by_delta`, the same test as before), as does a relation whose
reads come out unknown.

## Measured

All of PostHog (29,885 files, `len-_py.rr`), cold store, profiling build, an M3 Max, K = 15, every run unsampled.
The load average was 14–26 from other work, so compare runs that ran back to back:

| | wall | `resolve_export` | `sym_of__sup0` | cross part | the frames (main) | violations |
|---|---|---|---|---|---|---|
| morsels, run 1 | 42.0 s | 1.02 s | 0.74 s | 4.85 s | 0.68 s | 29,747 (483a6f116876) |
| morsels, run 2 | 42.2 s | 1.09 s | 0.73 s | 4.81 s | 0.71 s | the same set |
| morsels, run 3 | 42.8 s | 1.00 s | 0.75 s | 4.82 s | | the same set |
| `RERAIL_MORSELS=off` | 43.2 s | 1.15 s | 1.02 s | 5.03 s | 0.79 s | the same set |
| before §4 (text joins in morsels) | 44.9 s | 2.69 s | 0.71 s | 6.99 s | | the same set |

The plain join `sym_of__sup0` gains 28%. The cross part's chain gains 0.2 s, which is small: its two largest
strata read texts and still walk whole.

## Open

- **The atom arena's reads take a lock.** That keeps `resolve_export` and `sym_of` on one thread. The arena is
  append-only, so a reader could find a text by its id, and an id by its text, with no lock: segments published
  by an atomic length, and a lock-free hash table. Then §4 goes.
- **The tail after the cross part.** After `write_frames`, about 1.4 s remain before the process ends: the
  writer drains its frames and the process ends. This is not yet traced.
- **The main thread's Begin and Finish of each chunk** (decision 0254, Open) are still serial.
