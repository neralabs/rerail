# 0297: An order's price is the executor's count of its work

Status: built (2026-11-16). Refines the planner's step pricing (0038, 0203) and the maintenance estimate
(`keeps::calibrated_rows_at`, 0099). `logic/plan.rs` (`plan_search`: `step_all`, `priced`, `close`, the fixed-order
`price`, the subset search), `logic/plan/body.rs` (`Body::filter_pass`), `logic/plan/keeps.rs`.

## Intro

The planner chooses a rule's join order by its price: the rows of work it expects the order to do. The executor
counts the work an order did (`profile::plan_work`): for each step, the rows that enter it, and the last step's rows
out. Budgets, cuts (0206), trials (0207), slow lines (0203) and the check for a mispriced plan all compare one with
the other, so the two have to count the same thing.

They didn't. On the 300-file bench (317 rules, `rerail-shared/bench/step-vs-main`), the work done was 1.60 times the
price, and lines were off in both directions:

```
// A scan and three tests: priced at the scan's rows (the tests were free), ran 3.9 times that.
type_bare($n, $n) :- kind($n, $k), $k != 1232, $k != 1249, $n < 6917529027641081856.
// Two joins, no tests: priced at 1.6 times what it ran (a row between two joins counted twice).
```

| line | work ÷ price, before |
|---|---|
| one join, no filters | 1.00 |
| 2–4 joins, no filters | 0.60–0.62 |
| one join and 1, 2, 3, 5 filters | 1.88, 2.42, 3.85, 4.97 |
| 2 joins and 1, 3, 5 filters | 1.24, 3.12, 4.17 |

There were three causes:

1. **A join cost its rows in plus its rows out.** The executor counts a row once, as it enters the next step, so a
   row between two joins was priced twice.
2. **A filter cost nothing.** A negation, comparison or assignment is a step the executor counts too.
3. **A filter halved the rows.** Measured, filters pass far more than half (below).

## Decision

1. **Each step costs the rows it passes on**, a join and a filter alike. An order's price is then the executor's
   count: the rows entering each step are the step before's out. For a body of joins only, the last step's rows
   are the same whatever the order, so the cheapest order is the same as before; only the price's scale changes.
   The maintenance estimate (`calibrated_rows_at`) counts the same way.
2. **A filter passes its kind's measured share** (`Body::filter_pass`), taken from the profile's rows in and out per
   step (`native.tsv`, 9,307 plans), rows weighted:

   | filter | rows in | passed | priced |
   |---|---|---|---|
   | bound of an id's range (`$n < 6917529027641081856`, a constant of 2^30 or more) | 96.5 M | 0.976 | 0.98 |
   | negation (`!kind(…)` 0.953, any other 0.959) | 73.6 M | 0.958 | 0.95 |
   | `=` that binds a variable no join of the body binds (a call 0.993, a constant 0.903, a variable 0.956) | 38.7 M | | 1.0 |
   | `=` testing against a constant (`kind($e, $k), $k = 1205`) | 41.3 M | 0.624 | 0.6 |
   | `=` testing a call (`@prefix($x, 4492) = 1`) | 3.9 M | 0.391 | 0.4 |
   | `=` testing two variables | 0.2 M | 0.930 | 0.9 |
   | range test against another constant | 13.0 M | 0.706 | 0.7 |
   | `!=` | 8.1 M | 0.970 | 0.97 |
   | range test between variables | 2.6 M | 0.334 | 1/3 |
   | an aggregate | | | 1.0 |

## Measured

300 files × 317 rules, cold, `RERAIL_JOIN_SEED=1`, `RERAIL_JOIN_LINES`. All output relations have the same row
counts as before.

| | before | after |
|---|---|---|
| work ÷ price, whole run | 1.60 | 1.01 |
| work-weighted median of a line's work ÷ price | 2.95 | 1.02 |
| work-weighted 10th and 90th percentiles | 0.67, 5.26 | 0.86, 1.46 |
| share of work priced within 2× | 43% | 92% |
| join work, whole run | 552.9 M rows | 545.5 M rows |

The change in total work is within what one line's flip moves: `pth_deferred` (below) ran 0.26 M rows in these two
runs and 30 M in a third (with the kind pricing dropped below, which reads no literal of that rule), by taking its
other order at the same price.

## What CPU says about the count

A least-squares fit of each plan's CPU against its rows (5,870 plans with CPU, 51 s, the machine under load): a
join's probe about 124 ns, a join's row out about 72 ns, a row through a filter about 21 ns, and about 1.2 ms a
plan. So the count is a fair measure of joins, and weighs a filter's row about 4 times its CPU. Weighing the count
itself (the executor's budget unit) is left open: it would change every budget and threshold in rows.

## Tried and dropped

**A kind test priced at the kind's share of the nodes.** `kind($e, $k), $k = 1205` would pass 1205's nodes over all
nodes. The rows that reach such a test are mostly narrowed to a few kinds already (`$k = 1122` passed 463,779 of
463,782 rows), so the share is far too small: those lines' work came to 91 M rows against 59 M priced, and the work
priced within 2× fell from 92% to 85%. The class's measured 0.6 stays.

## Open

- **`within` read from its container** (`within($f, $s)` with `$s` bound): priced at 0.24 M rows, ran 30 M in
  `pth_deferred($n, $s) :- demand_pth_deferred($n, $s), look($n, _, $s), rr_nearest_ks4($n, $f), within($f, $s)`.
  Work queue Q33 (Q27 on cost-classifier before the phased-facts merge).
- Joins still mispriced by their statistics: `region`'s `comp_kind($ck, $k), kind($c, $ck)` (priced 10 M, ran
  0.17 M); `annotated_on_chain` (priced 0.03 M, ran 6.8 M).

## Tests

- `plan::tests::a_filter_is_priced_at_the_rows_it_passes_on`: a scan and two filters priced at the rows each passes
  on, with each filter's share.
- The planner's tests (`logic::plan::`) pass, except five that fail at 880f2a0de without this change (work queue Q32).
