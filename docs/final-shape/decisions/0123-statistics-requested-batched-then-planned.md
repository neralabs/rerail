# 0123: Statistics are requested, made in one batch, then planned with

Status: decided and built (2026-11-12): §2 (`plan.rs`: `Model::wants`, `wants_strata`, `make_statistics`, `late`;
`plan/issue.rs`), §3 (`plan/corpus.rs`, the kept relations' lazy statistics). It takes up 0122 §4. §5 is open.

## Intro

**What the planner's statistics are.** To price a join order the planner reads, per relation, its rows and, per
column, the distinct values, the fanout and the counts of the constants rules test. For a kept relation (the rows a
session's last run left, bound as `r__old`) these are made from its rows: counted from the engine's postings, or
measured. They cost real time: 2 ms to 0.2 s a relation.

**The problem.** At a session's first edit (O125 step 06) statistics cost 10.8 s, against 0.4 s of joins:

- **Made one relation at a time, as each stratum planned.** Each stratum made the statistics of everything its rules
  read before planning. Under the planning budget (0122 §1) most of its plans read none.
- **Made again at the end of the run.** Each maintained relation was counted once more (6.7 s), so that the next
  run could patch it by delta, whether or not a run would ever plan with it. `kind__sup0` was measured while
  planning (0.32 s) and counted at the end (0.84 s).

There is a second reason to change the shape: statistics will be estimated by a classifier on the device, in large
batches. A planner that asks for one relation's statistics in the middle of a plan cannot use one.

## 1. The shape

Planning is done in rounds. Each round has three steps:

1. **Request.** Each line the round will plan (a rule, with the atom that reads its delta, or none for a seed or
   whole plan) is asked whether it will be priced (`needs_statistics`). A line whose kept order is taken under the
   planning budget is not priced and reads no statistics: the budget test reads only rows, which are known before
   any statistic is made. Each line that will be priced requests the relations it reads whose statistics are still
   to be made.
2. **Batch.** The round's requests are made together (`make_statistics`), before any of its plans.
3. **Plan.** The round's lines are planned.

The rounds are:

| round | its lines | where |
|---|---|---|
| a level of strata | every stratum of the level, and the next level's that it chains (0100 §11): their seeds and probes | the strata loop, with the level's device wave |
| the components issued together | every member's seeds and probes | the scoreboard (0113), before the components start |
| a stratum's seeds | its seeds | `native_stratum_` |
| a stratum's probes | its probes and its whole plans, once its relations' rows are known | `native_stratum_` |
| a chunk's replans | the probes and whole plans replanned between chunks | `drive` |

The first two are the large batches; the stratum's rounds make only what a level missed. A line that reaches
planning with statistics no round made is made then (`late`), and counted. On the O125 replay this happens 0 times.

```text
plan cache: …; statistics: 95 batches of 711 relations, 0 late      (step 06)
```

**What the level's round leaves out.** Its strata have not run yet, so it asks only for the rules not known to
derive nothing (`runnable`): none may join a lower relation that is held with no rows, or an input delta bound
empty. A quiet stratum is one left with no such rule. A relation not run yet is not known to be empty, and stays in.

**Plans keyed as the stratum keys them.** A plan's key holds the component it is planned in (`solving_of`: the
stratum when a rule reads it). The level's seed joins (`seed_specs`) were planned with no component, so a recursive
stratum's seeds were planned twice and recorded no work under the level's key. They are now planned as the stratum
plans them.

## 2. Kept relations' statistics: counted when requested, then patched

- **No count at the end of the run.** A maintained relation is counted when a round first requests it, in the form
  that is patched by delta (`retain::Patched`), and patched at the end of every later run. `RERAIL_COUNT_AT_END`
  counts at the end as before (a measurement aid).
- **Counted from the postings, else from the rows.** The postings are read only where every column has them
  (`InRel::posts_all`). A relation that no later stratum read has its indexes dropped by the next run, and its count
  from postings failed after doing part of the work (0.07 s for `kind__old`, then a 0.18 s measure). It is now
  counted from its rows.
- **A maintained relation** is one with inserts (`r__ins`), not one with deletes (`r__del`): a levels stratum's
  relations have `r__gone`, not `r__del`, and were measured each step.
- A relation an earlier run measured is taken as measured when requested, not made at binding.

## 3. Measured

O125 replay steps 06–08, profiled, same as full: true. Statistics are the `count` and `measure` spans of the cross
part; planning is searches, re-prices and keys, without the profile's explain records.

| step | statistics, before | statistics, now | planning, now | joins, now | cross part, before → now |
|---|---|---|---|---|---|
| 06 | 10.82 s | 3.62 s | 0.91 s | 0.30 s | 19.0 → 10.6 s |
| 07 | 0.39 s | 0.17 s | 0.67 s | 2.27 s | 12.0 → 9.1 s |
| 08 | 0.14 s | 0.07 s | 1.22 s | 5.84 s | 17.5 → 16.0 s |

The cross part's seconds vary with the machine's load (15 to 40 here); the statistics do not move with it.

**The count is worth keeping.** Measuring the kept relations instead of counting them in the patchable form costs
2.04 / 1.61 / 1.77 s a step; counting costs 3.6 s once, then 0.1–0.2 s a step.

**Memory.** The process's largest footprint went from 2,008 to 2,275 MB, both above the budget. Measuring instead
of counting gives 2,102 MB, so about 170 MB is the patchable statistics kept for the session (they keep pairs per
document).

## 4. What a classifier on the device replaces

`make_statistics` takes a round's relations and installs their statistics. Its maker today is the host's, one
relation after another (`Deferred::statistics`). The batch boundary is where a device estimator replaces it as a
whole: the requests are known before any plan of the round is made, and nothing reads a statistic between the
batch and the plans.

## 5. Open

- **The first edit still plans more than it runs.** At step 06 statistics cost 3.6 s against 0.3 s of joins. The
  requests all come from lines with no kept plan (692 lines): the session's first run plans its maintenance rules
  for the first time. Two ways out: carry the cold run's plans into the session (it plans the
  rules that are derived again), or price a first-sight line from rows alone when its rows bound its work under a
  search.
- **A flaky test found on the way, not caused by this.** `closures_of_one_level_launch_in_one_wave` fails about one
  run in ten at the last commit too (4 of 32). With the scoreboard on (0113), the level's three closures sometimes
  run as component jobs on executor threads. There they cannot wait for the level's wave, and each runs its launches
  at once; the test counts waves on its own thread and sees none. So a level's launches are sometimes not one wave,
  which is a gap between 0113 and 0095 §5, not only in the test. `RERAIL_STRANDS=off` passes 8 of 8.
