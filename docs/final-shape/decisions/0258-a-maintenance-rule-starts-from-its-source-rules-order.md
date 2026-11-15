# 0258: A maintenance rule starts from its source rule's order

Status: built (2026-11-15). Takes up decision 0230's first open item: a session's first-sight plans. Option one of
0123 §5 ("carry the cold run's plans into the session"). `plan.rs` (`Model::started`, `source_order`,
`RunPlans::orders`), `plan/body.rs` (`Body::origin`, `Body::source_map`), `maintain.rs` (`write_ir`),
`program_image/build.rs` (`RuleBuilder::source`).

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
   - the source rule's order, for a maintenance rule. `source_order` leads with the line's delta, else with the rule's
     change: a join that maps to no source literal, or one over a maintenance relation of its literal's (not its old
     rows). The source's joins follow in their order, then the rule's other joins. Each filter goes in once it can
     run, as a search places them.

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

O125 replay (`spike_o125_replay`), one session step after a cold run at the step before. Debug build. Every run
ended "same as full: true".

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

With the start on, orders searched at step 12 fell from 5,672 to 4,881. On the debug build a search cost 125 µs
and a re-price 52 µs (2,779 and 564 spans at step 12).

## Open

- **The glue rules** (1,049 lines at step 12) have no source rule. They are a few literals over one relation's
  maintenance relations, the same shape for every relation. Their search is mostly its fixed part.
- **Across processes.** The line orders live with a session's plans in memory. A new process takes plans from the
  store by key (decision 0116), and its line orders start empty.

## Tests

- `a_maintenance_rule_starts_from_its_source_rules_order` (`plan/joins/tests.rs`): a form's overdelete
  `h__del(x, y) :- q(y), e__minus(x, y), !h__old(x, y)` of `h(x, y) :- e(x, y), q(y)`.
  - Its origin is `h`'s rule; a rule written with no source has none.
  - Its map is `[1, 0, none]`.
  - From the source's order `q, e`, it is led by `e__minus`, tests `h__old` once that binds its terms, then joins
    `q`. With a delta, the delta leads.
- Logic crate: `plan::` 134 (1 ignored), `native::` 52, `cut_tests` 4, `maintain` 3. Engine: `corpus::session` 15,
  `corpus::tests` 15, `logic::plan::tests` 55.
