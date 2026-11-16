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

## What it does not fix

- **Recursive strata miss by their join statistics, not by their rounds.** Q36 had blamed rounds: a probe priced
  for one round. After 0297, `walk` and `tgt` are priced within 2× of their work (1.53 M against 2.43 M, 0.08 M
  against 0.12 M). The large misses traced are in a join's fan-out or a seed's estimate:

  | stratum | planned | done | cause |
  |---|---|---|---|
  | `qualname` | 0.07 M | 0.97 M | `rr_nearest_scope($f, $s)` with `$s` bound fans 8,778 rows to 919,420 |
  | `qn` | 0.01 M | 0.10 M | a seed (`ibind`, `bound_in`, `walk`) did 72 K units |
  | the 50-relation `ask_*` stratum | 1,350 | 144 K | not yet traced |

  These are the planner's statistics. They are cost-classifier's to fix, after 0299, and are in the work queue.
- **Some views' rows cost far more than a unit.** `ann_root`'s seed reads `kind` and `field` at about 1.5 µs a
  unit. `fzk287`'s reads `line` and `col` at about 18 µs a unit. `rdefn` and `attr_of` read `text`. Their strata
  are priced at a quarter to a tenth of what they cost. A per-relation price for a view's row would cover them. The
  cost belongs to the view, not to placement: Q36 keeps it open.

## Tests

`placement::tests::a_stratum_is_placed_by_its_price_and_named_by_the_forms_the_device_lacks`: the copy that stays
on the CPU is now one run of `kind` as measured, 50,000 rows scanned and made.
