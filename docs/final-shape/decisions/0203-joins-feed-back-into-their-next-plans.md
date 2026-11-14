# 0203: What a run's joins measured feeds their next plans, first on the critical path

Status: draft (2026-11-14). The memo, the choice, the slow flag, the critical path, the wider search and its
upkeep job are built (`logic/plan/joins.rs`). Open: a batch pricer to set (O166), and searches that outlive their
process (O167).

## Intro

The planner orders a rule's body from statistics: each relation's rows, each column's distinct values and
fanout, and the fanouts observed for the orders that ran. A price can be wrong by orders of magnitude. Two
columns may be correlated, or a fanout may have been observed for another order than the one priced.

Before this decision the planner had these:

- **One exact search, keeping only its best.** `plan_search` is a dynamic program over subsets of the joins,
  greedy past 12 of them. Each subset keeps only its cheapest order, so no runner-up survives the search.
- **Replanning within a run.** Between chunks, a probe whose statistics moved, or whose fanout was observed, is
  searched again (`replanning::Watch`).
- **Replanning across runs.** A kept order priced under a search's work that ran over it is searched again
  (`mispriced`, 0123 §7).

These were missing:

- A record of which joins ran slower than priced.
- Which joins the run's time depends on.
- Statistics beyond the cheap ones, measured where they matter.
- Any order but the price's cheapest ever being tried.

The user asked for the planner to take feedback from the program's run "and RELENTLESSLY improve the critical
path" (2026-11-14), in seven parts. This decision builds each:

1. Metadata kept for a later exploration.
2. Decisions made in a batch, after a batch prediction.
3. A join that runs much longer than it should, flagged while it runs, with dearer statistics and a replan.
4. The joins on the critical path marked.
5. A background search and dearer statistics for the critical path.
6. A memo of that analysis, for better choices next time.
7. Randomized search, so that less likely join shapes are tried too.

## Decision

The parts below share one memo of lines. A *line* is a rule, its delta atom and the component it is solved in.
It is keyed by its plan key (`reused_key`), the same key its kept plans are under.

### 1. The search keeps its runners-up

When the dynamic program reaches the full set of joins, it prices one candidate per last join. Each is the best
order of the other joins, then that one. The best is the plan. Up to four others are kept with their prices
(`joins::searched_put`), and the line's choice reads them (§2).

The statistics a plan read are already kept with it (`Priced::sizes`).

### 2. A round's choices are made together

A line's choice (`Model::decide`) weighs these candidates:

- the search's best and its runners-up;
- the orders a wider search proposed for the line (§5), priced again now;
- a wider search's orders, made inline where the line's measured work is at least 100 times that search's cost
  (decision 0122: planning never costs more than running).

`joins::choose` takes a whole round of such choices:

- **One batch prediction first.** A `Pricer`, when one is set, prices every candidate of the round in one call.
  Without one, the planner's estimates stand.
- **The cheapest corrected price is chosen.** A candidate's price is multiplied by its order's measured ratio
  (§6), measured work over priced work. An order never tried keeps its price.
- **Exploration (§7).** A line that matters may now and then take an order it has not tried.

Rounds:

- Between chunks, the probes searched again are planned first, all in one round (`joins::Round`), then chosen
  at once, then applied.
- A line planned alone is a round of one.
- A line with no note and no pricer takes the search's best, with no other work.

### 3. A slow join is flagged while it runs

After each chunk, the stratum's driver (`Run::drive`) checks a probe against its price. It checks once the
probe's work passes `RERAIL_JOIN_SLOW_ROWS` (100,000 rows), then again each time the work doubles, so the checks
cost a fraction of the work.

**The check.** The probe's order is priced for the delta rows it actually read (`Model::price_for_delta`).
Work past `RERAIL_JOIN_SLOW` (8) times that price is slow. Bodies of one positive atom are not checked: their
order is forced. A slow line:

1. **Is noted:** a slow count, its price and its work.
2. **Has its dearer statistics measured: exact pair statistics.** The engine takes the rows of each positive
   atom past its constants, counted by its join columns (`setfix::step_histograms`). These are every live row,
   read at a stride past 2^20. For each two atoms that share a variable, it counts:
   - the rows of the first;
   - the rows of the second those rows join, Σ_v count_a(v) · count_b(v).

   That is an observation of the second atom bound by the first's variables, keyed as `observe` keys them. The
   search reads these first, before any observed fanout (basis `exact pair`).
3. **Is planned again now.** Its probe is marked dirty (`Watch::force`) and skips the planning budget once.
   Exact statistics change the rule's plan keys (the rule's epoch is in `plan_key`), so no kept price stands.
4. **Has a wider search asked for (§5).**

### 4. The critical path

**Recording.** Each stratum records each line that did at least a search's work (`SEARCH_WORK`) as it ran:

- its stratum;
- its order;
- its work;
- its price for the rows it read. Only lines past 10,000 rows are priced, so recording costs no search.

**The path.** At the run's end, `joins::critical` weights each stratum by its lines' work. The longest chain of
producers is the critical path. Strata come in dependency order, so it is one pass.

**Marking.** A line on the path with at least `RERAIL_JOIN_CRITICAL` (5%) of its work is noted:

- its share, averaged over runs;
- its order as tried, with its measured ratio.

A wider search is wanted for it where it has no proposals yet, or its order measured over twice its price.

Lines noted earlier have their orders recorded too, wherever they ran. The waiting wider searches of the path's
lines move to the front of the queue.

### 5. The wider search, in the background

`Model::explore` searches a line wider than a run does, with the statistics as they are (exact pairs included):

- the exact search up to 16 joins (`WIDE_GREEDY`; a run's is 12), never taking the greedy order first, with its
  runners-up;
- 48 random connected orders (`joins::random_orders`), from the delta where the line has one, each priced as a
  kept order is.

The best six, distinct, are the line's proposals.

**Where it runs.** A slow line, or a line of at least 100,000 rows of work, asks for one
(`Run::request_search`). The request holds:

- a model of the relations the rule reads (`Model::fork`);
- the rule;
- the program, shared once per run.

The `join-search` upkeep job (decision 0190) runs the queue at idle, most important first. Its starve bound is
five minutes.

Where a process has no idle time, the line's next plan makes the search inline, once its work pays for it (§2).

### 6. The memo

`joins::Note`, per line:

| Field | Meaning |
|---|---|
| `runs`, `slow`, `critical` | times on the path, times flagged, share of the path (moving average) |
| `est`, `ran` | price and measured work when it last ran |
| `tried` | up to 8 orders, each with its measured ratio (a geometric mean over runs) |
| `proposals` | up to 6 orders from a wider search, with their prices then |
| `wants_search` | a wider search is wanted, not made yet |
| `exact` | the rule's exact pair statistics |

**Where it lives.** In the process (at most 4,096 lines, the least important dropped first), and in the store
beside the kept plans: the same entry, relations `rr_join_note` and `rr_join_exact` (`choices.rs`). It is read at
the run's start with the plans, and written at its end with them, also where only the memo changed. A line this
process noted keeps its own over the store's (memory is ahead of the store, as decided).

**A pending line is searched.** A line with a wider search wanted, or a proposal not tried yet, is searched at
its next plan. Its kept price is not taken.

### 7. Exploration

A noted line that matters (on the path with its share, or slow) tries, at `RERAIL_JOIN_EXPLORE` (10%), an order
it has not tried. The order is drawn from its candidates whose corrected price is within `RERAIL_JOIN_MARGIN`
(2×) of the best's. What the order measures is recorded at the run's end (§4), so an order that wins is chosen
from then on, by its ratio. One that loses is not tried again, because it has been tried.

`RERAIL_JOIN_SEED` fixes the draws. `RERAIL_JOINS=off` turns all of this off. `RERAIL_JOINS_LOG` writes each flag,
choice, proposal and critical path to standard error. None of these settings changes what a run outputs, so all
are left out of the run-reuse key.

### Also fixed: upkeep is behind the run

Under `RERAIL_SQL_STRICT`, the SQLite job's first probe (`PRAGMA page_count`, on its own connection) ended a run
whose waves had started. Every upkeep job's `due` and `step` now mark their thread behind the run, as the
store's writer is (decision 0104).

## Measured (2026-11-14, debug build, strict)

**Unit tests** (`plan::joins`, 7 tests):
- the rows read back as written;
- random orders are distinct and connected;
- the critical path is the heaviest chain;
- a choice is corrected by its measured ratio, and a round chooses at its end;
- exploration stays within the margin;
- exact statistics turn the plan (`h(x,z) :- a(x,y), b(y,z)`: `a` leads on its statistics, `b` on the exact
  pair);
- a noted line takes the other order at its next plan, with a wider search made inline;
- a requested search proposes its orders.

**Persistence:** the memo is kept in the store, in the plans' entry (`choices` test).

**Planner tests:** 113 pass. **Native engine tests:** 48 pass.

**Smoke runs** on the 215-file Python fixture:
- With a three-way self-join rule (a function's calls, returns and identifiers through a recursive
  `inside`), the critical path was 5 of 8 strata and 1.80 M rows of work. One line was noted, and three wider
  searches were asked for.
- A policy's run (the collision rule) flagged two one-atom rules (string splits, 2–3 k rows of work against a
  price of 271). Such rules are no longer checked: their order is forced.

## Open

- **O166, the batch pricer.** `joins::Pricer` is the hook for the misprediction classifier (the cost model's
  batched GPU prediction, decision 0100 §14). None is set yet.
- **O167, searches that outlive their process.** A one-shot command exits before upkeep finds the machine idle.
  Its queued searches are lost; only `wants_search` is kept. The line's next run searches inline where its work
  pays for it. A daemon runs them at idle.
- The orders proposed are priced, never run off the run's path. Running them over stored facts, under a time
  bound, would measure them without a run's exploration. It waits on a store of a run's facts that a job can read.
