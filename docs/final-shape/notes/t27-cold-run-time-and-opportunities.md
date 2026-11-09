# t27. Where a cold run's time goes, and what each change could save

## Intro

This note measures one workload and sets out what could make it cheaper. The workload is `calls_cx` with
typeshed over the 84-file fixture (267 documents with the stubs it loads), from a new store, in the dev build.
The dev build optimizes the dependencies but not the engine's own code; see t25 for its figures against the
optimized build. Every run below agrees with Jedi on all 6,299 answers.

CPU is user + sys, on a shared machine at load averages 70–200, so two runs of the same code can differ by a
quarter. Compare large differences only, or runs made side by side.

## What changed (t25 and this note)

| step | first measured | now | the change |
|---|---|---|---|
| moving the chunks' rows to the cross-file part | 10.5 s | 6.3 s | a run of one document's rows finds its owning files once; rows gathered by file |
| storing the import components' rows | 3.5 s | 3.9 s (noise) | one sort of rows tagged by component |
| statistics phase | 11.9 s | 5.5 s | the stored plan's facts are not parsed back (3.6 s); base rows measured from their column slices; the file-local split found once per push |
| the 53-relation stratum | 20.5 s with guards | 18.4 s | with no import component found, the `rr_cross_doc` guards are dropped (side by side, same load) |
| main thread, whole run | 91.1 s (t25's prime) | 75.2 s | |

## Where the 78 s go now

From the last profiled run (main thread 78.2 s with the profiler's own cost; one worker thread parses, 2.0 s):

| | CPU | what it is |
|---|---|---|
| joins, cross-file part | 28.9 s | the native fixpoint over calls, values and bindings; 43.7 M join work units |
| joins, file-local part | 11.0 s | the chunks' rules over each file's tree |
| joins, typeshed's load requests | 5.1 s | the cone of rules that decides which stubs to load, in 21 pauses |
| moving rows to the cross-file part | 6.3 s | writing exports to SQLite, statistics totals, memo attribution |
| engine open and indexes | 4.2 s | loading inputs and building the joins' indexes |
| storing import components' rows | 3.9 s | the cross-file memo (decision 0076): grouping, closure checks, blocks |
| other host work in the chunks | 9.6 s | guests 1.9 s, measuring sources 1.2 s, per-chunk planning 0.8 s, the rest spread thin |
| statistics phase | 5.5 s | measuring the held base rows 2.6 s, pushing file-local joins 1.1 s |
| other | 4.8 s | the cross part's own glue 3.6 s, writing outputs 0.5 s |

The joins are 45 s, about 58%. Everything else is host code that moves, measures or indexes rows.

## The opportunity space

### 1. Not deriving what no answer reads (t26)

The liveness spike (t26) found 50% of the run's 5.94 M rows dead. Weighting each rule's join work by its head
relation's dead share gives a first estimate of the work spent on dead rows: **63% of the cross-file part's
43.7 M work units** (27.3 M). The largest shares:

| head | join work | rows | live | work on dead rows |
|---|---|---|---|---|
| `bv` | 9.26 M | 198,892 | 17% | 7.69 M |
| `pv` | 6.28 M | 397,775 | 37% | 3.97 M |
| `wbind` | 2.74 M | 23 | 4% | 2.62 M |
| `py_try` | 1.69 M | 1,605 | 0% | 1.69 M |
| `asks` | 2.37 M | 17,016 | 52% | 1.14 M |
| `decval` | 1.24 M | 1,143 | 19% | 1.00 M |
| `fret` | 2.39 M | 5,389 | 58% | 0.99 M |

Read with care:
- Work is not proportional to output rows. `wbind` makes 23 rows with 2.7 M units, so its probes are what cost.
  Demand would cut them only if it binds the probing literal early.
- Demand is not free. Its `need_` relations cost joins too. The saving is real only where few keys are live:
  `bv` (19% of keys), `found` (4%), `name`, `gname` and `ownq` (under 1%).
- `py_try` costs 1.7 M units and no answer reads a row of it. It is reached only through rows that are
  themselves dead, which relation-level pruning cannot see.

If demand saved half of the dead work, the cross-file joins would fall from 29 s to about 20 s. The rows moved,
stored and indexed would fall with them: moving, storing and indexing are 14 s, and they scale with rows.

### 2. Host overhead that scales with rows

About 33 s of the main thread is not joins. Most of it handles every row once or twice: writing exports, measuring
them, attributing them to files, grouping them by component, building indexes. In the dev build each such pass
costs a debug check per element. The cheaper shapes are:
- **Measure once, where the rows are made.** The chunks measure their exports (`totals`), and the statistics phase
  measures the held base rows again (2.6 s). For node columns the per-chunk figures add up (a node's values stay
  within its document), so the second pass could become a sum.
- **One columnar pass instead of several row passes.** Export to cross reads each export three times (write, totals,
  attribute). One pass over the columns could do all three.
- **Plan once, not per chunk.** Every chunk plans its strata again (0.8 s for 7 chunks, more with more chunks).
  The chunks share a program and their statistics differ little.

### 3. Typeshed's part

The stubs are 51% of the rows (3.0 M) and 41% of them are live. That is about the overall share, so pruning
does not single them out. What helps is what 0083 P1 gives: typeshed run to its fixed point in its own phase,
kept across runs, handing on only its exports. A cold run then pays for the workspace and the stubs it reads.
A warm run pays for the workspace alone.

### 4. Not measured here

- The optimized build. t25 measured it 5–6× faster on this workload (19.1 s against 117 s), more on host code
  than on joins. The ranking above could change there.
- Warm runs. They replay the memos (t25: 23.6 s dev, 4.4 s optimized), so their cost is replay into SQLite.

## Open items

- Demand per read, chosen from stored liveness (t26 § The on-demand path). Start with `bv` and `found`, which are
  selective, and with `wbind`, whose cost is in its probes.
- Base-row statistics summed from the chunks' instead of measured again.
- One columnar pass for exporting to the cross-file part.
- The joins' own costs in the dev build (`RowRef::at`, `Stored::rows_of`, index sorts): to be judged in the
  optimized build before any change.
