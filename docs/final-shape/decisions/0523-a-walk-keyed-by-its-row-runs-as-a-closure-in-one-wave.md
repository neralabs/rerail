# 0523. A walk keyed by its row runs as a closure in one wave: the climb W's lookup does, from the rule

Status: accepted (2026-11-21), the CPU form built. t50 step 5, first part; extends 0098 §1.

## Intro

The scope lookup climbs from a read's first scope until a scope binds its name:

```
walk(n, x, s) :- nameref(n, x), wstart(n, s), !tp_ref(n, x).
walk(n, x, p) :- walk(n, x, s), !bound_in(s, x), !gbefore(n, x, s), nearest(s, p, lfscope).
walk(n, x, p) :- walk(n, x, s), cls_late(n, x, s), nearest(s, p, lfscope).
```

W does it per reference: follow the scope's parents, probing each scope's bindings of the name.

The engine has a form for a linear recursion that walks one column along an edge, its closure (decision 0098 §1).
`walk` did not take it, for two reasons:

- the step's rest reads the row's other columns (`x` in `!bound_in(s, x)`; `n` and `x` in `!gbefore`), so a node's
  successors depend on the row, and there is no reach per node;
- there are two recursive probes along the same column.

So `walk` ran semi-naive, a scope level per wave. On s1000 that was 37 runs of the recursive line, each with a wave's
runs, delta and lazily built indexes.

## Decision

1. **The shape** (`closure::walk_of`). A component of one relation whose recursive probes each:
   - read the relation's delta first, with a distinct slot in every column;
   - make the same row with one column replaced, the same column in every probe;
   - read the relation nowhere else.

   A probe's rest may read the row's other columns. With one probe whose rest reads only the walking column, it is
   0098's closure as before. Otherwise it is **keyed**.
2. **The run** (`closure::keyed`), in the closure's first half, with no launches:
   - the seeds, as for any closure;
   - then a worklist by depth. Each probe's rest runs over the rows the last depth made, with all of a row's slots
     bound (`run_join_each_row`: one join context per probe and depth);
   - the rows it makes go into the relation, whose push is its set check. The new ones are the next depth's.

   That is one wave of the component, at any depth. An edge that is not functional, or a cycle, gives semi-naive's
   rows, since a row already made is not walked again.
3. **Levels.** A relation that keeps levels (decision 0090) is left to semi-naive, which gives each row its depth.
4. `RERAIL_KEYED_CLOSURE=off` leaves the keyed shape semi-naive, for measuring.

## Test

`a_keyed_walk_along_an_edge_runs_as_a_closure_in_one_wave` has the scope walk's shape: a keyed negation, a second
probe that climbs on where a test of the whole row holds, and a read that starts in two scopes. Its rows are a search's
and semi-naive's. It runs in one wave, where semi-naive takes one per level.

## Measured

`all` (3,089 files), the same host as the run before (st_pos read in place):

| | before | after |
|---|---|---|
| instructions | 3.264 T | 3.253 T |
| user CPU | 284.8 s | 280.1 s |
| lookup stage (rules) | 12.92 s | 12.61 s |
| `walk`'s own CPU | about 5.4–5.7 s | 5.4 s |

Every stage's rows are as before: the known `look`, `ref` and `open_name` differences of t50, and no others. Across
all programs, 74 components on s1000 took the keyed form, at most 5 depths deep. Two kept levels and ran semi-naive.

So the rounds were not `walk`'s cost. On s1000 its keyed steps take 422 ms of CPU, about what the recursive line took
semi-naive. Each row of the climb probes `bound_in`, `gbefore` and the nearest axis, about 200 ns each in the
executor. W's climb probes a scope's hash map of names.

## Next (t50 step 5)

- `walk`'s rows are still made: 4.9 M on `all`, almost all at the first scope. `look` and `miss` then read them. The
  operator's second part answers a read's scope at the climb's end, so the climb's rows are not made.
- The climb's probes are the cost. They are the executor's probes into sorted runs. A probe per scope and name, as
  W's, is the structure to build. A hash order was declined for its memory (t50, 2026-11-21).
- Metal: the device refuses a keyed closure. Its form is t50's segmented scan by key, once the shape is whole.
