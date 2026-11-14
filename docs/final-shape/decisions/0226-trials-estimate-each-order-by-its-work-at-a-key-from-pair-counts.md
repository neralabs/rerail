# 0226: A stratified trial estimates each order by its work at a key, from pair counts

Status: built (2026-11-14). Closes O173 of decision 0223: `KeyModel` and `Run::key_model` in `plan.rs`, used by a
checked line's stratified trials (decision 0221 §1).

## Intro

A trial runs each candidate order over a few rows of a line, and the planner scales that work up to the line. Where
the line's key is skewed, the sample is split in two: rows whose key is heavy, and the rest (decision 0221).
Decision 0223 then scaled each part by its rows' sizes: a key's rows, raised to the self-joins on it.

On the self-join seed `call_ret`, the estimate stayed at 0.69 of the line's work. The seed's rule:

    call_ret(f, n) :- inf(f, n), kind(n, "call"),
                      inf(f, m), kind(m, "return_statement"),
                      inf(f, k), kind(k, "identifier").

On its winning order, a return's work is about its function's calls times its function's nodes. Two functions of
one size can hold very different numbers of calls. A key's rows alone cannot see that.

## Decision

### 1. A key value's counts, filtered (`KeyModel`)

For a rule and its key variable, made from the engine's relations (`Run::key_model`):

- **Rows.** For each positive atom holding the key: its rows per key value (`inf.0`: a function's nodes).
- **Pair counts.** For each such atom whose other variable is checked by a filter: its rows per key value whose
  variable passes (`inf(f, n)` with `kind(n, "call")`: a function's calls). A filter is another positive atom
  with one variable and constants for the rest.

**Cost.** One pass over each atom on the key, counted once per relation and column, so a self-join's three atoms
share it. Then the filter's passing values, by its constant column: a lookup where an index holds the rows, else
one pass. Then one pass over the atom.

### 2. An order's work at a key value (`KeyModel::work`)

The order's steps after the lead are walked with the bindings a lead row has, starting at one:

- an atom on the key, once the key is bound, multiplies them by its rows at the value;
- a filter on such an atom's variable multiplies them by its share at the value: its pair count over the atom's rows;
- any other step is taken at 1;
- each step adds the bindings it reads.

This is the planner's step-by-step price, with per-value counts in place of a column's average.

### 3. Each order is estimated by its own sizes

A row's size is now its order's work at its key: one order's sizes are not another's. For each order and each part,
its work is scaled as:

    the sample's work × (the order's sizes over all the part's rows / over the rows it ran)

Without a model (an atom's relation not the engine's), the sizes of decision 0223 are used.

## Measured (2026-11-14, debug build, strict)

**Tests.**

- `plan::joins::tests::a_key_values_work_from_pair_counts_is_its_run`: two functions of 40 nodes, with 2 and 30
  calls. The model's work at each is the work of running the order on it by hand, exactly, and they differ by
  over 8×.
- Planner tests 120 pass, engine 50.

**Smoke** (`call_ret`; 545 findings in every run):

| Estimate | The winner's work per row | Share of the line's 874 |
|---|---|---|
| At a stride (0216) | 492 | 0.56 |
| Weighted by the parts' rows (0221) | 593 | 0.68 |
| By a key's rows (0223) | 599 | 0.69 |
| By each order's work at a key, from pair counts | 848 | 0.97 |

The winner and the line's work (385 k rows) are as before.

**Planning against running.** The model took 29.0 ms, and the split it needs 13.5 ms; the trials took 62.2 ms.
The line's 385 k rows of work run many times that. A first build, which counted each self-join atom again and
sorted `kind` for each filter, took 1.5 s; it is replaced.

## Open

All three addressed by decision 0232 (2026-11-14): pair counts are replaced by sampled walks through the order.

- **Filters are one variable and constants.** A filter joined on two variables, or a chain of two atoms from
  the key, is taken at 1.
- **The lead is taken to reach its key once.** A lead row whose link gives it several keys sums them.
- **The time on the large repository is not measured.** There, `kind` holds millions of rows. The filter's
  values come from its constant column's index where the engine holds one; on a source without one, the pass
  is over the whole relation.
