# 0352. A slow line's measure is kept, and acted on

Status: built (2026-11-17). Amends decisions 0203 §3 (a slow line is flagged and planned again), 0206 (the handoff)
and 0271 (a slow line is bounded on every path). Work queue Q96.

## Intro

The planner prices a join's order from statistics, and some prices are wrong by hundreds of times. Decision 0203
built the safety net for that: a join whose work passes its price by `slow_factor` (8) is *flagged* while it runs.
Its rule's exact pair statistics are then measured on the engine's rows, and it is planned again with them.
Decisions 0206 and 0271 bound the replacement: it runs under a limit, and where it is cut too, the order with less
work per row goes on.

On hogql (437 Python files, 317 rules, cold) that net fired 138 times in a run, and changed almost nothing:

```
joins: slow seed pre_546(...): 102049 rows of work, priced 513 for its 2663 rows read; 14 exact pair statistics
joins: slow seed pre_546(...): 102710 rows of work, priced 400 for its 10 rows read; 14 exact pair statistics
...                                                       (32 times for pre_546, 32 for pre_541, 16 for pre_540)
```

Each trip measured the exact statistics again, planned the line again, found the same order, and let it run on with
no further check. Three faults:

1. **The flag forgot its own line.** A line's plan key digests its rule's exact-statistics *epoch*, so that new exact
   statistics price its kept plans anew. The flag measures them, so it changed the line's key: the next chunk's plan
   of the line was under a key with no flag and no note, was checked again from nothing, and tripped again. Worse,
   the epoch was digested into *every* key once any rule had exact statistics (as 0): the first measure in a process
   keyed every line of every rule anew, and their kept plans and notes were not found.
2. **What the line measured was not kept.** The flag recorded that the line was slow, not what its order measured
   (work over price). The next choice (`joins::choose`) prices each order by its measured ratio where it has one, so
   it priced the slow order at its statistics' price again.
3. **The same order was not replaced.** Where the exact statistics planned the order already running, nothing was
   done: the exact statistics had not explained its work (it was still priced at 400 for 102 K), yet it ran on
   unchecked (0271 lists this under "Not done").

## Decision

1. **A line's note is kept under its key without the epoch.** A plan key carries its rule's epoch only when the rule
   has exact statistics (`plan_key_at`); the planner names such a key's line (`joins::line_of`), and the memo keeps
   every note, flag, check, trial and request under that line key (`joins::line`). Kept plans still take the epoch:
   new exact statistics re-price them as before.
2. **A flag keeps the order's measure.** `flag_slow` keeps the running order's work over its price as an order tried.
   As a cut trial's (decision 0267), it is a floor: it raises the order's kept ratio, never lowers it. The line's next
   choice prices that order at its measure.
3. **A flag holds for the order it flagged.** A flagged line's checks stop only while it runs that order. Another
   order of the line (a later chunk's choice) is checked again, so no new order runs unchecked.
4. **A same-order replan hands off to a runner-up.** Where planning the line again finds the order it runs, the
   search's runners-up (priced for the rows left) are compared with what the running order measured: its work per
   lead row done, times the rows left (at least its work again, where it finished none). The least priced under that
   goes on (`joins::over_measured`), under the handoff's limit. A runner-up priced as wrongly costs that limit's work,
   and the handoff then puts the old order back (0271).

## Measured

hogql, cold, profiled, 317 rules, on the m3 at loads of 23-61, one run before and two after, with `RERAIL_JOINS_LOG`:

| | before | after | after |
|---|---|---|---|
| slow flags | 138 | 61 | 59 |
| exact statistics measured | 106 | 33 | 33 |
| same-order replans handed to a runner-up | 0 | 4 | 5 |
| flags of `pre_546` / `pre_541` | 32 / 32 | 6 / 6 | 6 / 4 |
| work of `pre_540` to `pre_549` | 26.0 M | 17.8 M | 18.4 M |
| work of every stratum | 1.41 G | 1.33 G | 1.41 G |

Of the 4 to 5 runner-ups, `rr_agg56`'s and `p204_type_alias_read`'s went on; `pre_545`'s and `pre_540`'s made no row in
12 K of work and the old order went back, as 0271 has it. CPU is not compared: the machine's load moved more than the
change. The whole run's work moves within this noise; the strata the net was built for did less work.

Tests: `joins::tests::a_slow_lines_measure_outlives_its_exact_statistics` (rerail-logic); the joins, plan, corpus,
handoff and trial tests pass.

## Not done

- A cold run trials no line: decision 0274 trials only lines on a previous run's critical path. A cold run's slow
  lines are planned again (0271), never trialled.
- The runner-ups are priced by the same statistics that mispriced the running order; a replacement priced as wrongly
  is bounded by the handoff, not avoided. A learned correction of the planner's estimates (the misprediction
  classifier) would price them better.
