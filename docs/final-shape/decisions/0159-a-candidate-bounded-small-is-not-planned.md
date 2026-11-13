# 0159. A candidate its inputs bound under SMALL rows is stored with nothing planned

Status: built (2026-11-13). Narrows how decision 0099 estimates what it prices.

## Intro

Decision 0099 decides, per derived relation, whether to store its rows or recompute it inside each reader
(inline it). It prices each candidate from the planner's estimates. A candidate the planner estimates under
`SMALL` (1,024) rows is stored unpriced: its rows cost little either way.

Even so, every candidate was estimated by planning each of its rules. Each rule upstream of it was planned too,
since a candidate's rows depend on its inputs' rows. On the data-layer batch (44 policies, 100 programs chosen
for), that planned 6,213 of the programs' 8,330 rules. Of the 1,897 candidates priced, 1,854 came out small.
`keeps::choose` was 10.9% of the main thread, and `estimate`'s plans were 85% of it.

## The bound

A relation's rows are distinct head tuples. So without planning, a rule makes at most:

- the product, over its head's terms, of each variable's most distinct values (a constant counts one). A
  variable's most distinct values are the fewest of any column binding it in a positive atom;
- the product of its positive atoms' rows. This does not hold when an `ast` match binds: one match finds many
  nodes in a document.

An input's rows and distinct values are counted (`input_stats`, from the store's column statistics). A relation
of the program is bounded by its own rules, in evaluation order. A relation's bound is the sum of its rules'
bounds, and so is each column's, at most its rows. The following are unbounded: a recursion, a relation with
facts, and an input whose statistics are not made yet (lazy).

```
// b has 1,000,000 rows, 10 distinct x. c has 1,000,000 rows.
r(x) :- b(x, y), c(y, z).    // at most 10 rows, whatever the join's estimate
o(x) :- a(x), r(x).
```

## What changes

- **No plans for these candidates:** a candidate bounded under `SMALL` is stored small (`Priced::Small`). It is
  not a candidate for `estimate`, so neither its rules nor what only it reads are planned. The profile counts them:
  "N candidates stored small, their inputs' counts bounding them under 1024 rows: not planned".
- **Choices can change:** a candidate the planner estimated at `SMALL` rows or more, but whose inputs bound it
  under, used to be weighed and could be inlined. It is now stored. That estimate was over what the inputs allow.
  On the batch this stored 7 programs' relations that were inlined before, with the same 48 findings.

## What was tried and not kept

- **Greedy order for rows only** (each rule ordered by the cheapest connected step, no search). Its rows differ
  from the search's, because containment is applied step by step. That changed about ten programs' choices, and it
  saved only 4.5% of the batch's instructions: most of a plan's cost is per plan, not in the search.
- **The bound as a small upstream relation's rows**, in place of its plan. It saved 2.6% more, but it changed 4
  programs' choices by overstating their rows. The bound is sound as a test of small, not as an estimate.
- **Sharing plans across the batch's programs**: only 20% of the plans repeat a rule with the same statistics.

## Measured

dl1 batch, strict, load 15-19 (so instructions, not wall time): 152.0 → 148.8 G instructions, the same 48
findings. Plans: 6,213 → 3,407 rules for 434 candidates (was 2,445). `keeps::choose`: 10.9% → 8.7% of the main
thread. What remains is mostly searches of long bodies upstream of the remaining candidates (7 or more joins:
about 0.36 s of `estimate`'s 0.55 s).

## Tests

- `keeps::tests::a_relation_is_bounded_by_its_inputs_counts`: head variables, constants, an atom product, a
  comparison, an `ast` match (unbounded; written as a failing test first), a recursion.
- `keeps::tests::a_candidate_bounded_small_is_stored_unplanned`: the planner estimates `r` over `SMALL` and it
  would be inlined (checked with the bound off), and it is stored.
