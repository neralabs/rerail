# 0177. A warm batch recomputes nothing

Status: built (2026-11-13). Follows decision 0168 (a batch's policies run as one pack). Uses the whole-run
cutoff (`corpus::reuse`) and the combined program's store entry (`corpus::combined`).

## Intro

A batch that runs again over a tree that has not changed should only read what it stored. A profile of the
`dl1` batch (one Python file, 44 data-layer policies run as one pack) showed that every warm run did most of
the cold run's work again. Two caching paths were meant to prevent that, and neither worked in `run-policies`:

1. **The combined program was never found in the store.** Combining the 44 programs into one (0.57 s) was
   repeated on every warm run.
2. **The whole-run cutoff was never asked for.** Planning and evaluation ran again on every warm run, even
   though the inputs had not changed.

All figures below are from `--profile profiling` builds, as instructions retired (`/usr/bin/time -l`), at load
24–47 on the machine.

| `dl1`, 44 policies | before | after |
|---|---|---|
| cold, no store (`RERAIL_TAPE_CACHE=off`) | 27.4–29.3 G | unchanged |
| first run, filling a store | 44.1–44.6 G | 44.8 G |
| warm run | 14.65–15.0 G, 1.40 s user | **1.87–1.88 G, 0.13 s user, 0.37 s wall** |

The findings of every warm run are identical to the cold run's.

## Decision

### The combined program is pinned until its run names it

`combined::remember` writes the combined program's entry as soon as it is built. The run's hold
(`HeldPlan::new`) comes later, once planning starts. In between, nothing kept the entry's sources. A store
collection earlier in the same batch deleted them, because they were older than the current generation and no
pin or manifest named them. So the next run found no entry and combined everything again.

```
run k:   combine 44 programs -> remember(entry)            # sources written, not pinned
         ... a collection (generation + 1) ...             # sources unnamed -> deleted
         HeldPlan::new -> manifest names the entry         # too late
run k+1: lookup(entry) -> None -> combine again (0.57 s)
```

`remember` now pins both sources (the program and its text) under the owner `combined:<key>`, with a 6 h
lease. The run holds a `combined::Pins` until it has recorded its outputs. Dropping it releases the pins, and
by then the manifest names the entry. The test
`a_combined_program_outlives_a_collection_before_its_run_names_it` runs two collections between `remember`
and the lookup. It fails without the pins.

### `run-policies` asks for the cutoff

The command line's runs keep their outputs for the whole-run cutoff (`with_reuse` inserts `"reuse": true`).
`run-policies` passes no options (`null`), and `with_reuse` only edited an object, so the cutoff traced
`not wanted (asked false, ...)` for every batch. Now `null` is treated as no options, `{}`, so it gets the same
default. A caller's explicit `"reuse": false` still wins.

## What the profile showed, and what stays open

The profile covered the cold batch, from both the top (spans) and the bottom (sampled self time). These are the
levers that remain. None of them affects a warm run any more.

- **The run is serial.** The main thread does 3.19 s of 3.55 s of process CPU, and the workers only 0.15 s.
  The largest parts are preparing the program (1.26 s), file-local evaluation (0.64 s), combining (0.56 s)
  and the cross-file part (0.26 s).
- **Planning costs as much as running** in local evaluation. "Statistics and plan" takes 0.27 s of the 0.58 s,
  mostly in `keeps::choose` → `plan_search` and `calibrated_rows_at`. This works against "planning ≤ running"
  (decision 0122).
- **Pushing file-local joins splits the program 8 times** (0.30 s): each round splits all of its relations
  again (`typing::file_local`: `split_explained`, `split_memo`, `split_in`, `split_constants`, about 15% of
  the main thread).
- **Simplify's congruence** takes 9% of the main thread. `memcmp` is 15% of self time, called from simplify,
  `split_constants`, `split_in` and Tarjan's algorithm. That points at comparing whole rules and terms where an
  interned id would do.
- **Magic adornment recomputes strata** (`magic_adorn` → `demand_bound_columns` → Tarjan), 7% in `front`.
- **Loading the 44 programs** takes 0.12 s, spread over lowering, analyses, aggregate demand and cycles. Two
  attempts to share that work were built and dropped:
  - lowering library modules once per batch (the 57 lowerings were identical, but the gain was in the noise);
  - checking cycles once on the combined program (0.026 s, against 0.022 s for the 44 separate checks).
