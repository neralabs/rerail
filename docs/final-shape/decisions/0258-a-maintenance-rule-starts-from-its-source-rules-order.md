# 0258: A maintenance rule starts from its source rule's order

Status: built (2026-11-15). Takes up decision 0230's first open item: a session's first-sight plans. Option one of
0123 §5 ("carry the cold run's plans into the session"). `plan.rs` (`Model::started`, `source_order`,
`RunPlans::orders`), `plan/body.rs` (`Body::origin`, `Body::source_map`), `maintain.rs` (`write_ir`),
`program_image/build.rs` (`RuleBuilder::source`).

Kept (the user, 2026-11-15): source orders stay. They are an escape hatch from inefficient rules. cost-classifier's
withdrawal of this decision (49297f11e, "plan: remove source orders") is not taken when that branch is merged.
django-bugs' form (efcbd0717: tried inside the search, `RERAIL_SOURCE_ORDERS` opt-in) is the form the hub takes.
Off by default on data (django-bugs' "Re-validated" and "The start inside the search", O125 sessions 12-16): the
plans are the same (join work at parity, "same as full: true" either way), and on plans for more: 47.7 ms against
40.7 ms of first-sight planning thread CPU, alternating rounds; 65.5 against 59.0 ms in the start-first form. The
rule's own work moves by noise only (34.19/33.98/34.43 G off, 33.82/34.19/34.19 G on). Nothing measured shows the
case it is kept for, a rule the search plans badly; `RERAIL_SOURCE_ORDERS=on` is the hatch there.

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
6.5 µs and a start 3.3 µs (step 13: 1,891 searches in 27.8 ms). A started line saves half a search.

**The first form was a loss.** It led each seed with its change and followed the source's joins in their order.
The sessions came out 0.3% to 2% over the searches, though join work read lower, for two reasons:

- A seed led by its change could run a selective constant probe after it. `call_target__sup2__del` led with
  `callee__minus` before `bv($at, 29, 2992, $f)`: 21,558 rows against the search's 4,489.
- The source's sequence after a new lead could cross a join in. `tgt__held` read `patkind($k)` before `fl391_tgt`
  bound `$k`: 164 rows against 52.

Both are fixed above: a seed keeps the source's lead, and the next join is a connected one.

## Open

- **A plan call's fixed cost is the larger part.** A `plan #` span's own time, outside its search, is about 18 µs a
  line on the optimized build (step 13: 57.7 ms over 3,128 lines, against 27.8 ms of searches). That is the kept
  plan's lookup, `live_sizes` (each column's distinct values and fan, and each constant's count), the planning
  budget and the bookkeeping. It is paid by every line, kept or new. Starting from the source's order does not reach it.
- **The glue rules** (1,049 lines at step 12) have no source rule. They are a few literals over one relation's
  maintenance relations, the same shape for every relation. Their search is mostly its fixed part.
- **Orders priced over a search's cost** are priced and then searched (414 at step 12). That is a start's cost
  spent for nothing.
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
