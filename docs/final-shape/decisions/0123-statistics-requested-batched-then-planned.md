# 0123: Statistics are requested, made in one batch, then planned with

Status: decided and built (2026-11-12): §2 (`plan.rs`: `Model::wants`, `wants_strata`, `make_statistics`, `late`;
`plan/issue.rs`), §3 (`plan/corpus.rs`, the kept relations' lazy statistics). It takes up 0122 §4. §5's two items
are closed (2026-11-13): §6 (`plan/issue.rs`, `lanes::wave`) and §7 (`plan.rs`, `mispriced`, greedy first). §8
(`profile`, jobs' spans) and §9 (`plan.rs`, `native/setfix.rs`: lazy probes) are built (2026-11-13).

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
  that is patched by delta (`retain::Patched`), and patched at the end of every later run. A relation whose
  statistics the engine kept as its rows came (`InRel::kept_stats`) is read, not counted. (`RERAIL_COUNT_AT_END`,
  which counted at the end as before, is gone since the merge into investigate-bend-vs-datalog.)
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
  search. **Closed (2026-11-13), in two parts.** The statistics went with the merge of the hub: the engine keeps a
  relation's statistics as its rows come (`InRel::kept_stats`, 7bfd96d03), so a round finds them made. Step 06 now
  makes 22 relations in 9 batches, 0.00 s (711 relations, 3.62 s before). The searches stayed: §7.
- **A flaky test found on the way, not caused by this.** `closures_of_one_level_launch_in_one_wave` fails about one
  run in ten at the last commit too (4 of 32). With the scoreboard on (0113), the level's three closures sometimes
  run as one component's job on an executor thread. **Closed (2026-11-12):** the job still flushed them as one wave
  of three (a component's members share a wave, `run_members`); only the count was the job thread's, and the test
  read the run's. A job now moves its waves to the thread that takes it back (`wave::since`, `wave::credit`, carried
  in its `Strand`): 30 of 30 with strands on. So it was not a gap between 0113 and 0095 §5. Strata of one level in
  different components (`RERAIL_STRATA_MERGE=off`) still flush a wave each.

## 6. The components issued together share one device wave

**The gap.** 0113 issues a level's strata as components, each run as a job. A component's members share a wave
(`run_members`), but each component flushed its own. So a level split across components went out as one wave per
component. `closures_of_a_level_wider_than_a_component_launch_in_one_wave` (20 closures, more than one component
holds) flushed 3 waves for 20 launches.

**Now.** A component's job runs its members to their launches and returns. The members still waiting are parked,
with the launches queued on the job's thread (`wave::take_queued`). The run's thread takes each job of the same
`issue` back (`Component::group`) and requeues its launches (`wave::requeue`). The group then goes out as one wave
(`flush_group`). Each parked component is resumed with its launches' outputs (`wave::take_outputs`, `put_outputs`):
one that ran as a job is resumed as a job, one that ran on the run's thread is resumed there. Members that wait
again go out with the next wave. The test flushes 1 wave of 20 launches.

- **Tickets are the process's** (`wave::NEXT`, an atomic). A job's launches are flushed on another thread, so a
  thread's own count would give two launches one ticket.
- **A job hands over only what it queued** (`wave::mark`). A thread waiting on a job runs others meanwhile
  (`Stream::wait` helps the pool), so what is queued on it is not all one job's. Taking all of it lost a parked
  component's launches (`launch 0 has not run`, on the O125 replay).
- **Device joins stay on their thread.** A device stratum encodes its joins into its thread's open command buffer
  (`setfix::device::submit`, a deferred submission), which cannot move. A job that opened one flushes its members'
  wave itself (`wave::deferred_since`), as before.
- **Resumed members reserve their atoms** as `run_members` does (`Strand::reserving`, 0114 §4).
- `RERAIL_LEVEL_WAVE=off` flushes a wave per component, as before (a measurement aid).

**Measured.** On the O125 replay a step has one waiting stratum (1 wave of 1 launch, in both modes), so the wave
count does not move there. The cross part's retired instructions are at parity (41.5 / 58.7 / 67.3 G for steps
06–08 with the switch off, 41.6 / 57.2 / 65.9 G on), same as full: true. The run thread's spans moved more, because
resumed members ran on it where the jobs ran before. Until §8 the profile missed a job's work.

## 7. Greedy first, held to what it did

**What remained at the first edit.** With the statistics gone (§5), step 06 still searched 4,710 orders. Of its
4,297 lines, 4,073 did fewer than 250 rows of work (1,509 none), so most of the searches could not
pay.

**Now.** An order is taken greedily, with no search, where its priced work is under what a search costs
(`search_rows`). This is 0132 §3's greedy first, now on by default. 0132 kept it off because its actual regret was
six times its estimate, and it named a bound on actual regret as what would make it safe. Measured work gives that
bound from one run on: a kept order priced under a search's cost whose line did a search's work or more when it
last ran is searched at its next plan (`mispriced`, counted as `searched: ran over its price`). The search then
costs less than the line ran, by 0122's rule. The guard holds whether the order came from greedy or from a search.
Until the line runs again, the new plan is not held to the old work.

`RERAIL_PLAN_GREEDY_FIRST=off` searches as before. `RERAIL_PLAN_CACHE_COUNTS` prints the orders searched and those
taken greedily.

**Measured.** O125 replay steps 06–08, the cross part's retired instructions, same as full: true:

| | cold (05) | step 06 | step 07 | step 08 |
|---|---|---|---|---|
| search always, run 1 | | 40.90 G | 56.36 G | 63.83 G |
| greedy first, held to what it did, run 1 | | 40.13 G | 54.66 G | 63.13 G |
| search always, run 2 | 76.00 G | 41.03 G | 56.69 G | 64.15 G |
| greedy first, held to what it did, run 2 | 76.26 G | 40.95 G | 54.96 G | 63.09 G |
| orders searched, search always (run 2) | 1,948 | 4,710 | 3,197 | 1,949 |
| orders searched + taken greedily (run 2) | 1,523 + 425 | 2,723 + 1,991 | 1,836 + 1,434 | 1,529 + 421 |

So the session's steps save 0.2–3.1%, and the cold run is at parity (+0.3%). The orders are counted over the
process (`RERAIL_PLAN_CACHE_COUNTS`).

**Still over at step 06.** With jobs in the profile (§8), step 06 spends 0.54 s searching and keying (4,573
searches) against 0.41 s of joins and 0.15 M rows of work. Steps 07 and 08 are within the rule (0.31 s against
1.85 s; 0.27 s against 1.71 s). Greedy first takes only bodies of three joins or more. Most of step 06's searches
are smaller bodies, whose search is the fixed cost alone.

**Not bounded: an order's first run.** Before a line has run, nothing measured holds it; only its price does. In a
cold run every line is in its first run, so the cold run's regret is 0132's: +1.3% join work against a search a
third cheaper.

## 8. The profile counts a job's work

**The gap.** A component's job (0113) runs on an executor's thread, which has no span open. Its spans and native
records opened at the profile's root, not under the run: at step 06, 805,547 rows of work were recorded under no
span, against 66,883 under the cross part. So the cross part's planning and joins, read from the profile, were the
run thread's alone. They read 0.01–0.02 s of planning at steps 06–08, which was not so.

**Now.** The run's `Ambient` takes the profile's path where it issues jobs (`profile::here`). A job entering it
opens its spans under that path (`profile::based`, a thread's base path, used when no span is open on it). Each
member a job runs is a span of its own, `stratum … (in a job)`, and `… (in a job, after its device wave)` when
resumed. So a stratum's work is profiled under the run, whichever thread ran it. Its CPU is its own thread's.
The run's span over it keeps its own thread's CPU, so a parent's total can be less than its children's.
`a_jobs_spans_open_under_the_run_that_issued_it` tests it.

**Measured** (O125 replay, profiled, same as full: true). Nothing is recorded under no span. The cross part's
`search` spans count 4,573 at step 06, against 4,714 orders the process searched or took greedily. In the cross
part, by step:

| step | search | re-price | explain records | joins | work | in jobs |
|---|---|---|---|---|---|---|
| 06 | 4,573, 0.52 s | 0 | 0.30 s | 0.41 s | 0.15 M | 3.40 s |
| 07 | 1,753, 0.23 s | 954, 0.05 s | 0.11 s | 1.85 s | 1.17 M | 4.62 s |
| 08 | 1,958, 0.18 s | 1,938, 0.07 s | 0.08 s | 1.71 s | 1.44 M | 4.04 s |

The numbers measured before this (§3 here, 0122 §1) read the run's thread. Where jobs ran, they undercount.


## 9. A probe is planned when its delta first has rows

**The gap.** At step 06 the cross part made 4,714 orders (searched, or taken greedily) in 0.51 s of planning,
against 0.29 s of joins. Of its 2,497 probes, 1,431 never had a delta row: a stratum planned every probe when
it opened, and most of a maintained component's relations never change in a small edit. Greedy first (§7) did
not help there: a greedy take still costs a call into the planner, and most of the cost is the call's fixed part.

**Now.** A probe with no kept plan to start from (`Model::first_sight`: it would be a first sight) is left
unplanned when its stratum opens. Its spec line is `unplanned h d` (`Line::Unplanned`). When a wave's delta for
it has rows, setfix returns before the wave (code `4`). The driver then plans the probes so due as one round:
their statistics in one batch (§2), then each plan (`Run::plan_probe_now`), and it gives each one's join to the
engine as a replan does (`Reprobe::Join`). The run goes on from the same wave. A probe whose delta never has rows
is never planned.

- **Where it is not done.** A component of one relation (`closure::recognize` reads its probe when it opens), a
  run answering requests, and a probe with a whole plan (it reads the relation's first delta in full) are planned
  when the stratum opens, as before. A probe with a kept plan is taken as before, from the cache, under 0122's
  budget.
- **On the device.** Placement compiles every probe when the stratum opens, so the probes left unplanned are
  planned there before the spec goes to the device.
- **Statistics.** A lazy probe's statistics are not requested in the stratum's round of probes. They are
  requested when it is planned.
- `RERAIL_LAZY_PROBES=off` plans every probe when its stratum opens, as before (a measurement aid).
  `RERAIL_PLAN_CACHE_COUNTS` prints the probes left unplanned at open and those planned later.

`a_probe_is_planned_when_its_delta_first_has_rows` (`plan/tests.rs`) tests it. A component `{r, s, t}` in which
`s` never has a row: the probe of `r` by `s` is never planned. The probes by `r` are planned when the first wave
returns, and the outputs are the SQL run's.

**Measured** (O125 replay, two alternating runs each way, debug, same as full: true in all four). The cross
part's retired instructions, by step:

| step | eager, run 1 | eager, run 2 | lazy, run 1 | lazy, run 2 |
|---|---|---|---|---|
| cold at 05 | 75.81 G | 76.10 G | 76.82 G | 76.97 G |
| 06 | 40.33 G | 40.45 G | 35.81 G | 35.81 G |
| 07 | 55.06 G | 55.01 G | 52.00 G | 51.62 G |
| 08 | 63.39 G | 63.00 G | 62.43 G | 61.76 G |

From the profile of the second runs (planning without the profile's explain records):

| step | orders made, eager → lazy | planning, eager → lazy | joins, eager → lazy |
|---|---|---|---|
| 06 | 4,714 → 2,916 | 0.51 → 0.21 s | 0.29 → 0.27 s |
| 07 | 3,199 → 3,394 | 0.29 → 0.36 s | 1.15 → 1.36 s |
| 08 | 3,927 → 3,740 | 0.32 → 0.26 s | 1.32 → 1.44 s |

So step 06 now plans for less time than it joins. Steps 07 and 08 retire fewer instructions. Their seconds
moved with the machine's load (40 at the time), and the instructions are the measure there. At 07 a probe that
06 never planned is a first sight, and is searched there. The cold run retires about 1 G more: its probes are all
first sights, and each due round costs a return from setfix and a statistics batch.
