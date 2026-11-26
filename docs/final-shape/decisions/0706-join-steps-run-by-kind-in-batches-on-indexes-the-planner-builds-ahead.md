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

### Step 1, as built

- `logic::access` (in `rerail-logic-core`, which setfix and the planner both read; the planner's base sits above
  setfix): `Access`, `Kind`, `Available`, `Use`, and the price of a membership index (`build_ns`, `saves_ns`,
  `pays`).
- `blocks::relation::FILTER`: a relation's membership filter on several columns in its block, in `bloom`'s format
  (decision 0488), placed by `bloom::key` (`try_add_filter`, `find_filter`, `filter_may_hold`), given back with the
  block's other indexes. `facts::rows::InBlock::own_filter` finds one, or builds it for the build ahead.
- setfix `access`: `needs` (a membership step's `Access`: node bits for one column, a filter for several, sorted),
  `observe` (each stratum's steps' tests and finds, added to the run's `Observed` when `step_counts` takes them),
  and `prefetch` (the level's build ahead, in parallel: each observed `Access` on a relation the level reads whose
  use pays, built into its block). `RERAIL_INDEX_AHEAD=off` builds none.
- The walker and `cands::holds` read node bits, then the filter, before the exact probe. A reader only finds an
  index: node bits are no longer built at a key's first test (0694 step 1), and a reader that found none looks again
  only once its relation's builds ahead have moved (`InRel::ahead`).
- The driver gathers each level's reads (`reads_of`, from the rule bodies) and calls `prefetch_indexes` where the
  level's count tables are prefetched: a level, a maintained level, and issued components.
- §3 (the search pricing with `Available`) is not yet wired: the search prices orders in bindings (and the learned
  model, decision 0354), and a membership test is placed as soon as its columns are bound whatever index it reads,
  so a filter changes the step's cost, not the order. `Available` enters the search with E5, whose order indexes
  decide which binding step leads.

Measured on `all` (m1max, load 6–8, the head and this step built from one tree, two runs each, same rows): CPU 128.4
/ 131.5 s before, 128.7 / 129.1 s after; instructions 1,407 / 1,409 G before, 1,409 / 1,408 G after; peak 2,293 / 2,342
MB before, 2,342 / 2,333 MB after. No change.

Why (`RERAIL_INDEX_TRACE`): the step's indexes are built and read, but the membership tests that cost time are of
views, not computed relations. `kind` by two bound columns is 5,541 plans' steps (883 more negated), then `docof`,
`field` and `child`: E1's. Of the computed relations', most find their row (`rf_def`: 31,480 tests, all found), and
`nameref`'s misses (6.65 M tests, 22 k found, its filter built) were refused already by 0488's probe screen
(`join_filter`). So E2's filters save little on `all`, and the catalogue's worth is in the executors that follow,
E1 first.

### Step 2 (E1's tests), as built

- `Source::test_own_batch` (facts_base): for a batch of ids of a view's own column, whether each one's row has its
  wanted value in another column (one value for all, or one each). `Viewed` (syntax.rs) finds every id first
  (document, local id, tape node, row), then reads each, through the document's directory row where the view is the
  kernels', else off the tape, as `read_matches` reads a probe by the own column. The values wrapper (values.rs)
  then checks its made rows or its parts for the ids the base did not match, as its `lookup_read_at` does.
- setfix `kinds`: a step testing a view by a bound node and a second bound value (a constant or a slot: `kind(n,
  K)`, `!kind(n, K)`, `child(c, n, _)` both bound) is a `KindTest`. The walker leaves each row at the step
  (`Ctx::batch`); 256 rows, or the walk's end, are tested in one call, and the rows that pass go on. A waiting row
  counts as gone past every step (no factored step records it as failed). A batch finishes every row even past the
  walk's budget, which then stops the walk at its next step: a batch's rows are of first-step rows a cut counts as
  finished (dropping them lost rows, read under negation as 50 more findings; found by `RERAIL_KIND_BATCH=walk`).
  Not batched: the first step, a delta-restricted step, a screened step, or a limited walk that collects its rows.
- Closures (decision 0561) ask a kind test of a depth's rows with one batch call.
- `RERAIL_KIND_BATCH=off|walk|closure` and `RERAIL_KIND_TRACE` are measurement aids. Test:
  `kind_tests_in_batches_give_each_rows_rows` (tests_corpus), the rows checked against the source's own.

On `all` (m1max, load 7–10, two runs each, same rows): 18.9 M rows in 139,778 batches, 0.49 M in closures. CPU
129.6 / 127.6 s before, 124.3 / 124.1 s after; instructions 1,408 / 1,409 G before, 1,368 / 1,367 G after; peak
2,319 / 2,350 MB before, 2,291 / 2,341 MB after. W: 48.8 s, 477 G.

### Step 3 (E3, grouped aggregates), as built

What the scalar steps did on `all` (`RERAIL_SCALAR_TRACE`): almost every probe missed the pool's memo and folded
its group (`public_len`: 330,631 probes, 318,886 folds, a row a group; `rf_fn_at`'s count: 481,713 probes, 448,132
folds over 21,473 rows a chunk), each fold two write locks, a candidates lookup and its rows.

- `scalars::Pool::build_whole`: a bag's every group over its input's live rows, folded in one pass into each
  shard's memo for the input's version, marked whole. A probe of a whole memo reads its group, and a key with none
  is the empty group (count 0, no minimum): nothing is folded.
- The catalogue: a bag is `Kind::Summary` (its group columns, its tests and value column). `access::observe` adds a
  scalar step's probes and its input's rows (`Use { tests: probes, found: rows }`); `pays` prices a summary by
  probes a row held, a fold at 1,000 ns against a probe of a whole summary at 60 and a build at 30 a row.
- `scalars::build_ahead`, at each component run's start: every scalar step over a complete input (held, not a
  view) whose observed use pays, built whole in parallel before any walk. Readers fold only what is not built.
- Tests: `a_whole_summary_gives_each_groups_fold_and_empty_groups` (setfix), `a_summary_pays_by_its_probes_a_row`
  (logic-core), `aggregates_over_summaries_built_ahead_give_each_groups_rows` (tests_corpus, 120 files in chunks of
  64 nodes, the counts and minima against the source).

On `all` (m1max, load 9–14, two runs each, same rows): CPU 127.5 / 129.4 s before, 125.7 / 126.6 s after;
instructions 1,366 / 1,371 G before, 1,361 / 1,360 G after. Folds left: each bag's first chunk (`public_len` 48,380,
`import_start` 46,645), priced before any use is observed; the first chunk prices by the search's estimates with
§3.

## Consequences

- 0694's node bits are made by the build batch, not at a key's first test.
- `InRel`'s CSRs and the views' lookups made on first probe become builds the batch has done.
- The step-shape tags (`step_shapes`, `RERAIL_SPIKE_SHAPES`) stay as the profile that steps' executors are chosen
  from.
