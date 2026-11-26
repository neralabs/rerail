# 0706. Join steps run by kind, in batches, on indexes the planner builds ahead and shares

Status: decided (the user, 2026-11-26: "go ahead and build this", on the page "Specialised join executors" and its
section "Indexes: declared by the executors, planned ahead, shared"). Being built.

## Intro

The head is about 107 s of CPU on `all` against W's 49 s (m3, 2026-11-25), and the difference is the joins. A
line-level profile of the walker (`run_join_in_each`, 58 k samples) found no hot line: about 39% is the walker's own
generic code, the rest the reads it calls. A Rust executor written for a join shape, called when a plan fits it, removes
the generic part.

The run's plans, tagged by their compiled steps (`RERAIL_SPIKE_SHAPES`, step CPU 42.5 s over 232.6 M units), do not
concentrate by whole plan: 1,458 plans, 725 shapes, the largest 9%. They concentrate by step kind:

| Step kind | CPU | Share | ns a row in |
|---|---|---|---|
| Probe of a computed relation that binds | 6.66 s | 15.7% | 258 (`within` 1,108) |
| Tape view by node (`kind`, `field`, `node`, `child`, `docof`, `text`) | 5.51 s | 13.0% | 348 |
| Kind test against a constant, and its negation | 4.92 s | 11.6% | 230 |
| Membership in a computed relation (exists, not) | 6.30 s | 14.8% | 100 (`nameref`: 7.3 M probes, 0.03 M found) |
| Grouped aggregate (`min`, `max`, `count`) | 4.12 s | 9.7% | 1,474 |
| Seed scans | 4.50 s | 10.6% | `field` 660 a row out |
| A node's text | 2.19 s | 5.1% | 218 |

## Decision

### Executors by step kind

Each executor takes a batch of bindings (rows of the plan's slots, a column a slot) and does one step kind for the
batch, handing the rows that pass to the next. A plan is a chain of them. A step no executor covers runs as the
walker runs it. The rows are the walker's, and tests compare them.

- **E1, node reads**: a row's node resolved to (document, local) once a batch and kept beside it. A kind test
  against a constant is `kinds[code(local)] == K` from the entry's code → kind table, gathered and compared across
  the batch. Probes by node read the tape in place. A node's text reads the tape index's term atom (0656).
- **E2, membership**: a filter over the relation's key columns, checked for the batch before an exact probe. It is
  the node-keyed index (0694's `NODE_BITS`) for a node column, and a blocked Bloom filter on the key's hash
  otherwise.
- **E3, grouped aggregates**: one pass over the bag's source an epoch, its rows ordered by group key in the program
  and each group reduced in SIMD, into the scalar pool's summaries. The step then probes the summary.
- **E4, seed scans with a constant**: candidates from the column's value index where the source keeps one,
  otherwise the column filtered in place in SIMD.
- **E5, keyed probes that bind**: the batch's keys sorted (0112) and merged against the held order's ids. `within`
  gets an interval walk of its own.

They run inside the morsels the walker runs in now, on the same threads. Head writes stay as they are.

### Indexes: declared, planned, built ahead, shared

1. **One description of an index**: `Access { rel, kind }`, the kind being an order on columns, a membership filter on
   columns, node bits on a column, a value index on a column, a group summary (bag, group columns, aggregate), or an
   interval order.
2. **Each executor declares, for a step, what it needs and what it costs**: `needs(step)` and
   `price(step, rows, &Available)`, priced with each index built and in the step's fallback form without it. The
   walker declares its own probes the same way.
3. **The search prices with what is available**: per index, built, scheduled (its build cost split over the readers
   known to share it), or absent (the fallback).
4. **A level's indexes are chosen together**: its plans' needs gathered and made unique by `Access`. An index is
   kept when its readers' savings exceed its cost, as `keeps` decides a relation. A rule whose index was not kept is
   planned again at its fallback.
5. **Built ahead, in parallel, in blocks**: an input's and a view's when the chunk's documents are in, before the
   first stratum; a computed relation's when its last writing stratum finishes, before the level of its first
   reader (as `prefetch_tables` does for the device). A build is a job over the rows in place. Each index is written
   once into the relation's block (a derived index, 0353) and listed beside the relation. No reader makes one.
6. **Relations still growing**: a recursive relation's orders grow by waves, as now. Filters, bitmaps and summaries
   are only for relations complete before their readers.
7. **Observed use**: each index built records the probes it answered and what it saved. A later chunk weighs that
   over the estimates (as `keeps::observe` does). An index that saved nothing is not built again. One an executor
   wanted and did not have is counted and priced next time.

`Access` and `Available` live in the planner's base beside the statistics. `needs` and `price` sit with each
executor in setfix. The level's choice and its build batch live in the driver. In tests, an index made by a reader
mid-walk is an error.

### It is not a copy (decision 0353)

A batch holds the plan's slot values for its rows. These are bindings, which the walker holds in `slots` one row at
a time, not rows of a block. Every index is a derived index in its relation's block, written once.

## Steps

1. The catalogue end to end with E2: `Access`, `needs` and `price` for the walker's probes and E2's filters, the
   search's `Available`, the level's choice, the build batch, and E2's filtered membership.
2. E1, node reads and kind tests.
3. E3, grouped aggregates.
4. E4, seed scans.
5. E5, keyed probes and `within`.

Each step is measured W against the head on `all`, with the same rows, and its result written here.

## Consequences

- 0694's node bits are made by the build batch, not at a key's first test.
- `InRel`'s CSRs and the views' lookups made on first probe become builds the batch has done.
- The step-shape tags (`step_shapes`, `RERAIL_SPIKE_SHAPES`) stay as the profile that steps' executors are chosen
  from.
