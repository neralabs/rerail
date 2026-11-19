# 0304: Placement's CPU price counts the executor's unit

Status: built (2026-11-16). Follows 0297 (an order's price is the executor's count) and 0299. `logic/plan/placement.rs`:
`CPU_PER_WORK`, `CPU_PER_SCANNED`, `CPU_PER_STRATUM`, `CPU_PER_PROBE`, `CPU_PER_ROW_OUT`. Work queue Q36.

## Intro

Placement decides, per stratum, whether the CPU or the GPU runs it. It prices the stratum on each from the
planner's estimate of its work. Before 0297, that estimate and the executor's count were different units: a join
cost its rows in as well as its rows out, and a filter cost nothing. So placement's CPU prices were fitted to a
unit the planner no longer gives. After 0297, on the run below, the planned work is within 5% of the work done
(26.2 M planned, 25.0 M done). The prices per unit were still the old ones:

```
CPU_PER_WORK     100 ns   a unit of planned work (fitted 2026-11-09, the old unit)
CPU_PER_SCANNED   30 ns   a row a scan reads (0277: a copy of a stored input)
CPU_PER_STRATUM  0.13 ms  a stratum run's floor (open and close)
```

The CPU-placed strata cost 2.5 s against 1.0 s priced. Only 14% of their CPU was priced within 2×.

## Measured

PostHog's `posthog/hogql` (437 files), the light run (`len-_py.rr`), cold, under the Metal binding and
`RERAIL_SQL_STRICT=1`, profiled. `placement.tsv` now also gives each stratum's planned scanned rows and steps.

A least-squares fit of the 169 CPU-placed strata (2,395 runs, 2.5 s of CPU):

| term | fitted |
|---|---|
| a stratum run | 0.71 ms |
| a row a scan reads | 117 ns |
| any other unit of planned work | 52 ns |

With each run's steps as a fourth term, a step comes to about 8 µs and a run to 0.60 ms. A run has 11.7 steps on
average, so that is the same 0.7 ms, and steps are not priced apart.

- **A run's floor** holds its plans and its replans in the waves, not only its open and close. 0297's fit put a
  plan at about 1.2 ms, with the machine loaded.
- **A scan's row** is now a view's row read in place (0289, 0293), at 150–300 instructions a row (Q36). The 30 ns
  measured before was a stored copy's read.
- **Any other unit** is between 0297's 72 ns for a join's row out and its 21 ns for a filter's row.

## Decision

1. **The CPU's stratum price is the fit**: 0.7 ms a run, 117 ns a row scanned and 50 ns any other unit.
2. **A join step's CPU index price is 0297's fit**: 124 ns a probe (a row in) and 72 ns a row out. Before, it was
   100 ns for each row in and each row out, the double count 0297 removed from the planner.
3. **The device's prices are unchanged.** Its estimates were within 2–3× of its measured runs, with no bias (Q36).

## Results

The same run, before and after (the machine loaded, at 60–75). The run was priced at 0.6 ms a run; the floor is in
neither side of the comparison that places a stratum (`gpu_cheaper` compares the work alone), so placements are
the same at 0.7 ms.

| | before | after |
|---|---|---|
| strata's CPU, priced ÷ measured | 2.54 s ÷ 3.52 s | 3.17 s ÷ 3.22 s |
| `kind` (a copy, 66–71 runs) | 116 ms priced, 482 ms ran | 450 ms priced, 452 ms ran |
| `pre_119` (priced on the CPU) | 866 ms, ran 426 ms | 414 ms, ran 416 ms |
| strata placed on the GPU | 7 | 4 |
| instructions retired | 94.9 G | 81.1 G |

The three strata that left the GPU (`miss`, `rr_ctx15`, `xk_fz286_fl279_violation_3`) ran a few of their runs
there. Their CPU is the same either way, within 2 ms. The findings are the same: 1,629.

## A recursive probe's rounds (added the same day)

The first version of this record said the recursive strata missed by their join statistics, not by their rounds.
The explain of each probe says otherwise. A probe's plan is priced at the delta the planner sees when the stratum
opens: one round's, often the relation's few rows. The executor runs it over every row the relation gains, since
each row is a delta once:

| stratum | the delta priced (a plan) | the delta run (a loop) | the probe's fan-out, priced / run |
|---|---|---|---|
| `qualname` | 18 | about 500 | 107 / 103 rows a scope |
| `qn` | 6.4 | about 1,480 | |
| `walk` | 3,992 | about 18,400 | |
| the `ask_*` stratum | 1 | hundreds to thousands | |

`qualname`'s fan-out at `rr_nearest_scope($f, $s)` was priced right.

3. **A recursive probe is priced over the rows its relation gains** (`Shape::close_rounds`). The probe's planned
   work per delta row, times its relation's first delta and one round more. The first delta is the larger of what
   the seeds make and the delta the plan saw. The round more is at most as many rows again: a round's estimate is
   rough, and the rounds after it compound its error.

On the same run:

| stratum | planned before | planned after | work done |
|---|---|---|---|
| `qualname` | 80 K | 137 K | 903 K |
| `parsed_in` | 18 K | 39 K | 29 K |
| `ask_p0_*` | 57 | 109 | 373 |
| `exports/resolve_export` | 137 K | 259 K | 27 K |
| `exported/public_name` | 34 K | 43 K | 10 K |

The rest of the miss is in the seeds' estimates, which this count multiplies. `qualname`'s seed `def_name($f, $x),
rr_nearest_scope($f, $s), modname($s, $m)` is priced at 32.5 rows a plan and makes about 181 a loop: `modname` after
`rr_nearest_scope` is given a containment of 0.056, while about 30% of definitions sit at module scope. With the
seed right, the count gives about 71 K a loop against 52 K run. `exports`' seeds are priced over their rows, and the
count multiplies that too. `qn`'s seed (`ibind`, `bound_in`, `walk`) did 72 K units against 405 priced. These are
the planner's statistics: work queue Q44.

## What it does not fix

- **The seeds' estimates** above (Q44).
- **Some views' rows cost far more than a unit.** Two shapes were fixed after this record (Q36: a field probed by
  parent and field reads the tape; a column pushed in order is its own index). `attr_of`, `rdefn` and `fzk287`
  are still priced at a fifth or less of their CPU. The profile now gives each join step's CPU, summed by the
  relation it reads (`sources.tsv`): a view's lookup costs 4 to 27 units, `within`'s 176 (Q36). Placement does not
  price by relation yet.

## Tests

`placement::tests::a_stratum_is_placed_by_its_price_and_named_by_the_forms_the_device_lacks`: the copy that stays
on the CPU is now one run of `kind` as measured, 50,000 rows scanned and made; a recursive probe priced over its
seeds' rows and one round more, and one that makes more rows than it reads adding one round, not a divergent sum.
