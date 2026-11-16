# 0258: A maintenance rule starts from its source rule's order

Status: built (2026-11-15); off by default since the same day: a net cost in planning (see "Re-validated" and "The start inside the search"). Takes up decision 0230's first open item: a session's first-sight plans. Option one of
0123 §5 ("carry the cold run's plans into the session"). `plan.rs` (`Model::started`, `source_order`,
`RunPlans::orders`), `plan/body.rs` (`Body::origin`, `Body::source_map`), `maintain.rs` (`write_ir`),
`program_image/build.rs` (`RuleBuilder::source`).

Kept (the user, 2026-11-15): source orders stay. They are an escape hatch from inefficient rules. cost-classifier's
withdrawal of this decision (49297f11e, "plan: remove source orders") is not taken when that branch is merged.
django-bugs' form (efcbd0717: tried inside the search, `RERAIL_SOURCE_ORDERS` opt-in) is the form the hub takes.

## Intro

A session maintains a changed stratum with rules written from its rules (decision 0077 §3). For a rule
`h :- e(x, y), q(y)` it writes, among others, an overdelete for `e`'s position:

```text
h__del(x, y) :- e__minus(x, y), q__old(y).
```

The planner keys a plan by its body, its delta and the component that solves it (`plan_key`). A maintenance rule's
body is new to the session, so its first plan is a search. Decision 0230 counted 2,887 such first-sight plans in O125
session 14, about 12% of the session's stream jobs.

The cold run planned each source rule. A maintenance rule is its source rule's body with one position read as a
change, and the rest over old or new rows. So the source's order, led by that change, is an order the session can
price instead of searching.

## What changed

1. **A maintenance rule names its source rule.** `maintain::write_ir` records the source rule of each rule it writes
   from one (`RuleBuilder::source`, the image's §9 source, as `Builder::rewrite` records it). A form's rules name the
   run's version's rules, since the form is a derivation of it (decision 0241).
2. **The planner finds the source.** `Versions::at` gives a maintained form's rule the run's version, and
   `Body::origin` reads its source rule there. It checks that the head is one of the source's maintenance relations
   (`h__del` of `h`).
3. **Each literal is mapped to the source's literal** (`Body::source_map`), by relation and terms. The rewrite either
   copies a source literal, or writes one of its maintenance relations (`e__minus`, `q__old`) over the same terms.
   - An exact relation is matched first, then a maintenance relation of it.
   - What the rewrite adds (`h__old(..)`, `h__del(..)` of a rederive, an aggregate's group) maps to no source literal.
4. **Each rule line keeps its last order** (`RunPlans::orders`, keyed by `line_key`: the body and the delta, not the
   component). It is set where a line is searched or started. A session carries it between its runs, with its plans.
5. **A first sight starts from a kept order** (`Model::started`). There are two kinds, tried in this order:
   - the line's own order, kept under another key (the same body, solved in another component);
   - the source rule's order, for a maintenance rule. `source_order` leads with the line's delta where it has one.
     A seed keeps the source's lead: each literal stands where its source literal stood, since a seed's start is
     the search's to choose. The source's joins follow in their order, then the rule's other joins. Each next join
     is the first of them that shares a bound variable, while one does (no cross product while a connected join
     remains, as in the search). Each filter goes in once it can run, as a search places them.

   The order is priced (`plan_inner` with the order fixed). It is taken where it is led by its delta and priced
   under what a search costs (`search_rows`): a search could not pay for itself there (decisions 0122, 0123 §7). Else
   the line is searched, as before. A line with work queued for its next plan (`joins::pending`) is searched.
   - A started plan is held to what it does as any kept plan is: one priced under a search that runs over a
     search's work is searched at its next plan (`mispriced`).
   - It is counted as `started: its line's order` or `started: its source rule's order`
     (`RERAIL_PLAN_CACHE_COUNTS`). The profile's decisions say why a line was searched instead
     (`source orders (decision 0258)`).
   - `RERAIL_SOURCE_ORDERS=off` searches every first sight, as before.

## Measured

O125 replay (`spike_o125_replay`): a cold run at step 11, then sessions 12 to 16. Every run ended "same as full: true".

**What starts.** Debug build, one session step after a cold run at the step before:

| session | first-sight lines | started from the source rule | searched |
|---|---|---|---|
| 12 | 2,279 | 770 (34%) | 1,509 |
| 13 | 3,889 | 1,560 (40%) | 2,329 |
| 14 | 2,948 | 1,342 (46%) | 1,606 |

At step 12, the searched lines:

| why | lines |
|---|---|
| no source rule: the rewrite's glue (`h__minus :- h__del, !h__redo, !h__ins`, survivors, deltas) | 1,049 |
| the source's order priced over a search's cost (a search can pay) | 414 |
| the source rule's line has no order kept | 50 |
| the source's order not led by the delta | 2 |

**What it saves.** `--profile profiling`, unprofiled, the cross part's retired instructions (G), two rounds each.
The join choices' exploration was seeded (`RERAIL_JOIN_SEED=1`): with its clock seed, the same build's sessions
moved by up to 2% between rounds, and step 16's join work by a third.

| session | off | on | |
|---|---|---|---|
| 12 | 11.76, 11.80 | 11.98, 12.04 | +1.9% |
| 13 | 7.10, 7.24 | 7.13, 7.22 | 0 |
| 14 | 6.56, 6.59 | 6.40, 6.55 | −1.4% |
| 15 | 5.05, 4.94 | 4.82, 4.81 | −3.6% |
| 16 | 7.35, 7.35 | 7.06, 7.07 | −3.9% |
| the five | 37.87 | 37.54 | −0.9% |
| cold (11) | 10.44 | 10.47 | +0.3% |

The join work is at parity: at step 12, 5.46 M rows against 5.44 M. On the optimized build a search costs about
6.5 µs and a start 3.3 µs (step 13: 1,891 searches in 27.8 ms). A started line saves half a search. (Not so, as
measured per plan call: see "Re-validated" below. The table above was the first form's, and the saving noise.)

**The first form was a loss.** It led each seed with its change and followed the source's joins in their order.
The sessions came out 0.3% to 2% over the searches, though join work read lower, for two reasons:

- A seed led by its change could run a selective constant probe after it. `call_target__sup2__del` led with
  `callee__minus` before `bv($at, 29, 2992, $f)`: 21,558 rows against the search's 4,489.
- The source's sequence after a new lead could cross a join in. `tgt__held` read `patkind($k)` before `fl391_tgt`
  bound `$k`: 164 rows against 52.

Both are fixed above: a seed keeps the source's lead, and the next join is a connected one.

## Re-validated: what starts, and what it saves

2026-11-15, after the hub merge (646d533b9). `--profile profiling`, `RERAIL_JOIN_SEED=1`. An A/B worktree built
this decision's two commits with a diagnostic (each start's verdict, price and search cost; each plan call's thread
CPU in starts and searches), not committed.

**Sessions 12 to 16 are the full run's** ("same as full: true" at every step, two runs).

**The table of what starts was the first form's.** It was measured at ce8bc3dd2 and carried into f597b540c, whose
fix (a seed keeps the source's lead, the next join a connected one) changes the orders priced. Step 12:

| code | started | priced over a search's cost |
|---|---|---|
| ce8bc3dd2, the first form | 783 (34%) | 414 |
| f597b540c, as built | 704 (31%) | 493 |
| after the hub merge, with decision 0265 | 707 (31%) | 491 |

Matched by rule text across the last two (1,033 lines), 5 lines change verdict. The merge did not move it, nor did
0265 (708 without it). Steps 13 and 14 now: 1,409 of 3,923 (36%) and 1,271 of 2,745 (46%).

**It costs planning, not saves it.** Each plan call's thread CPU in `plan_rule_reused`, sessions 12 to 16, mean of
three rounds:

| | f597b540c off | on | merged off | on |
|---|---|---|---|---|
| searches | 69.8 ms (6,649) | 47.0 ms (4,444) | 58.2 ms (6,345) | 38.7 ms (4,093) |
| starts taken | | 15.9 ms | | 15.3 ms |
| starts refused | 0.9 ms | 14.2 ms | 0.8 ms | 11.5 ms |
| planning in all | 70.7 ms | 77.2 ms | 59.0 ms | 65.5 ms |

- A line that starts saves its search (about 20 ms of searches) for about 15 ms of starts: about 30% of a search,
  not half.
- A refused start (about 490 a step, priced over a search's cost) prices the source's order, then searches: 12 to
  14 ms for nothing.
- So source orders cost about 7 ms more planning over the five sessions, in both builds.

**Why a start saves so little.** A search costs 400 rows and one per candidate order (`search_rows`). Of step 12's
707 starts, 272 have one candidate order and 218 two: there a search prices what the start prices. And pricing one
order runs most of a search's set-up (each atom's binding domains, `delta_misses`, every join priced from no binding
for `SEED`). The gate (a price under a search's cost) keeps a start safe for the plan; it does not pick the lines
where a start saves planning.

**The −0.9% above was noise.** The sessions' instruction totals move more between rounds than planning can move
them: f597b540c off read 36.95, 38.08 and 37.03 G (3%), against about 0.03 G of planning. Merged: off 34.19, 33.98,
34.43; on 33.82, 34.19, 34.19. The join work is unchanged either way.

## The start inside the search, then off by default

2026-11-15. Two changes, then measured:

1. **The search tries the start first** (`SearchCx::start`, read back as `SearchCx::started`). `start_order` finds
   the order; the search prices it under its own set-up (`price`, the same loop a kept order is priced by) and takes
   it, before greedy-first, where it runs, is led by the delta and prices under a search's cost. A refused start
   then costs its steps, not a second set-up.
2. **No start where a search prices fewer than three orders** (`START_CANDIDATES`): there the search prices about
   what the start does.

Thread CPU of first-sight planning, sessions 12 to 16, off and on run alternately (two rounds, load steady):

| | lines | ms |
|---|---|---|
| off: every line searched | 6,325 | 40.7, 40.8 |
| on: in all | 6,332 | 47.7, 47.9 |
| of which finding start orders (`start_order`) | 2,950 | 5.2, 5.3 |
| starts taken (with finding them) | 1,503 | 10.1, 10.3 |
| starts refused (with finding them, and the search) | 1,446 | 18.5, 18.6 |
| searched with no start | 3,370 | 19.2, 19.0 |

Still 7 ms more, as before the change. Most of it is finding the order (the source's body, its literal map, the
mapped order, two lookups: about 1.8 µs a line). Without it the starts would still not save: greedy-first (decision
0123 §7) already prices one cheap order and takes it under a search's cost, under the same set-up, so a start saves
only greedy's other candidates.

So source orders are **off by default** (`RERAIL_SOURCE_ORDERS=on` turns them on): the plans are the same either
way (join work at parity, "same as full: true" with them on, steps 12 and 13), and off plans for less. With them
off, no order is kept for them either (`orders_put`).

## Open

- **A plan call's fixed cost is the larger part.** A `plan #` span's own time, outside its search, is about 18 µs a
  line on the optimized build (step 13: 57.7 ms over 3,128 lines, against 27.8 ms of searches). That is the kept
  plan's lookup, `live_sizes` (each column's distinct values and fan, and each constant's count), the planning
  budget and the bookkeeping. It is paid by every line, kept or new. Starting from the source's order does not reach it.
  Taken up by decision 0265: with the profile off it was about 7 µs a line (the rest was the profile's own
  formatting), and it is 4 to 5 µs now.
- **The glue rules** (1,049 lines at step 12) have no source rule. They are a few literals over one relation's
  maintenance relations, the same shape for every relation. Their search is mostly its fixed part.
- **Orders priced over a search's cost** are priced and then searched (493 at step 12, as built). That is a start's
  cost spent for nothing: 12 to 14 ms over sessions 12 to 16, more than the starts save.
- **A start where a search prices one or two orders** saves nothing: most of the lines that start.
- **Across processes.** The line orders live with a session's plans in memory. A new process takes plans from the
  store by key (decision 0116), and its line orders start empty.

## Tests

- `a_maintenance_rule_starts_from_its_source_rules_order` (`plan/joins/tests.rs`): a form's overdelete
  `h__del(x, y) :- q(y), e__minus(x, y), !h__old(x, y)` of `h(x, y) :- e(x, y), q(y)`.
  - Its origin is `h`'s rule; a rule written with no source has none.
  - Its map is `[1, 0, none]`.
  - A seed keeps the source's lead: from `e, q` it runs `e__minus`, then `h__old` once that binds its terms, then
    `q`; from `q, e` it runs `q`, `e__minus`, `h__old`. With a delta, the delta leads.
  - An insert of `g(x, z) :- a(x), e(x, y), c(y, z)` led by `a__plus(x)`, from the source's `c, e, a`: `e` (connected)
    before `c`.
- `walked_orders_are_costed_against_the_walked_order_that_ran` failed about one run in ten: a walk within the
  walks' spread is not taken on its walk, but an untried order within the margin is still explored at
  `explore_rate`, a clock-seeded draw. The test now admits the exploration, named as such.
- Logic crate: `plan::` 134 (1 ignored), `native::` 52, `cut_tests` 4, `maintain` 3. Engine: `corpus::session` 15,
  `corpus::tests` 15, `logic::plan::tests` 55.
