# 0232: A trial sizes a row by a sampled walk through the order, which measures checks, chains, cycles and triangles

Status: built (2026-11-14). Replaces decision 0262's pair counts (`KeyModel`) with `plan::Walker`, probing through
`InRel::probe_each`. Closes 0262's three open items.

## Intro

A checked line's trials run each candidate order over a few of the line's rows, and scale that work up to the line.
On a skewed key, decision 0221 split the sample into rows of heavy keys and the rest. Decision 0262 scaled each
part by an estimate of each row's work: per key value, the rows of each atom on the key, and their pair counts with
one-variable checks. It got the self-join seed `call_ret` to 0.97 of its line's work.

It left three things:

- **Checks on more than one variable** were taken at 1. That covers a chain (`inf(f, n), child(n, c),
  kind(c, "x")`), and an atom closing a cycle or a triangle (`e(n, j)`, over two variables that two other atoms
  bound).
- **On a large repository** it read every row of the atoms on the key, and every row of a check's relation where
  no index held its constant. `kind` holds millions of rows there.
- **Its time there** was not measured.

The user asked for a way to solve cycles and triangle joins.

## Decision

### 1. A walk of sampled bindings (`Walker`)

A row's size is its order's work, estimated by carrying a sample of real bindings through the order's steps after
the lead. The walk starts from the line's own lead rows: their bindings, read from the lead relation. At each step:

| Step | What the walk does |
|---|---|
| An atom binding new variables | Probes with the bound ones; the bindings multiply by the sample's mean matches; the matches are sampled again |
| An atom whose variables are all bound: a check, or the atom closing a cycle or triangle | Tests each sampled binding; the bindings multiply by the share that pass |
| A negated atom | Passes the bindings it does not match |
| Any other literal | Taken at 1 |

Each step adds the bindings it reads. This is the planner's step-by-step price, measured on rows rather than
averaged.

**Exact or unbiased.** Where the sample holds every binding, the walk is the order's work exactly. Where it holds
fewer, it estimates it without bias: each step's sample is uniform over the bindings that reach it.

**Rare checks are measured wide.** An expansion followed by checks samples 16 times as many bindings, so a check
passing 1% of them is measured on enough rows. The sample is thinned back to its width before the next expansion.

**Sampling.** A reservoir draws a uniform sample of matches while probing; the rest are counted, not kept. Draws
come from a seeded generator, so walks are repeatable. A stride over the relation's order was dropped: it was biased
where values cluster (a function's calls first among its nodes).

### 2. Probes go through the join's own index (`InRel::probe_each`)

The walk probes a relation the way a join step does (`run_join`'s candidates of `Src::In`): through its own index for
the probed columns, made once and kept with the joins'. A check such as `kind(n, "call")`, with `n` bound, is a
lookup of `n`, not a scan or sort of `kind`.

The key is put in the index's own column order, since an index is kept by its set of columns. The lead's links to
its keys are probed the same way (`heavy_parts`), not read whole.

### 3. Bounded

- **One budget a line:** 20,000 probes over all its orders and parts (`RERAIL_WALK_PROBES`). It counts the wide
  walks too.
- **Width:** 64 bindings a step (`RERAIL_WALK_WIDTH`).
- **Which rows are walked.** Rows are grouped by their key, and a group is walked from up to 4 of its rows. Groups
  holding a row the trial ran are walked first, at 4 times the width, since their sizes are the ratio's
  denominator. The rest are walked in a random order until the budget is spent. A group not walked takes its key
  rows' size, times the walked groups' ratio of walk to key rows.
- **A cut order is not walked.** It cannot win, so it is weighted by its parts' rows (decision 0221).
- **The log walks nothing.** It reads only sizes already walked.

## Measured (2026-11-14, debug build, strict, every run under `tmp/capped.sh`)

**Tests.**

- `plan::joins::tests::an_orders_work_by_a_walk_is_its_run_through_a_triangle`:
  - the walk equals a brute-force run of the order through a filter, a chain and a triangle's closing atom;
  - two functions of one size and one count of calls, with 90 and 3 edges among their nodes, have work 215 and
    128;
  - a narrow walk (16 bindings a step, of up to 90) has a mean over 400 walks within 5% of the work.
- Planner tests 120 pass, engine 50.

**The fixture's self-join seed `call_ret`** (545 findings in every run). The winner's estimate as a share of its
line's 874 a row:

| Sizes | Share | Walk time | Probes |
|---|---|---|---|
| Pair counts (0262) | 0.97 | 29 ms (pair counts) | — |
| Walks, stride thinning, narrow before checks | 0.70–1.02 (run to run) | 753 ms – 8 s | 30–51 k |
| Walks, reservoir, wide before checks, budget per order and part | 0.97 | 107 ms | 61.6 k |
| The same, one budget a line (built) | 0.97 | 38 ms | 21.5 k |

With the walk wide before checks, the heavy part's total converges as the width grows: 216,710 at 64, 217,550 at
256. The exact pair counts give 217,553.

Budget per order and part, measured before the budget became one per line:

| Budget | Share | Walk time |
|---|---|---|
| 100,000 | 0.97 | 107 ms |
| 20,000 | 0.93 | 46 ms |
| 5,000 | 0.62 | 26 ms |

**The data-layer repository**, every 17th Python file. Each run was capped at 1,000 MB and 115 s, and the steps
went 1 → 10 → 50 files:

- 1 file: 1 finding, 148 MB.
- 10 files: 475 findings, 164 MB.
- 50 files: 3,462 findings in 59 s. Every trial of its `call_ret` line was cut, so nothing was walked.
- 50 files with heavy values off: passed 115 s and was killed.
- Before the log walked nothing: the 50 files' log walked 165 k probes in 2.2 s for the running order's sizes, with
  `kind` at 7 µs a probe.

The larger step (328 files) is not run: it passes the run cap.

## Open

Decision 0233 (2026-11-14) replaces §3's grouping, group budget and key-row sizes with two-phase sampling.

- **Comparisons and assignments are taken at 1.** A variable bound by one (`w = x + y`) is treated as unbound by
  a later atom.
- **The walk's time on a line whose trials finish, at scale, is not measured.** On the 50 files every order was
  cut, so the trial cap may be small for this repository. The bound is the budget: 20,000 probes, about 140 ms in
  debug at the 7 µs a `kind` probe measured there.
