# 0320: The planner's estimates know a field's kinds, and its record prices filters as its search does

Status: built (2026-11-16). Follows 0299 (the planner's statistics) and 0312. `logic/plan.rs`: `Fielded`,
`Stats::fielded`, the `kind` and `field` estimates, `plan_search`'s record; `logic/plan/corpus.rs`: `Own::fields`.
Work queue Q44, Q36.
Numbered 0314 on this branch until the number was found taken in another worktree.

## Intro

The planner estimates each join step's rows, and placement (0304, 0312) prices a stratum from them: the CPU's price
by the work planned, the GPU's by the rows its joins walk. A stratum estimated at a tenth of its rows is priced at
a tenth of its CPU, and stays where it is.

On PostHog's `posthog/hogql` (437 files, the light run) the steps that make 100 rows or more missed their rows by
8.2× on average, weighted by rows, and a third were within 2×. Three causes made most of it.

1. **The plan's record halved each filter.** The search priced a filter by `Body::filter_pass` (0297): a bound of
   an id's value class (`$n < 6917529027641081856`, which every node passes) keeps 98%, a test of a constant 70%.
   The record placement reads (the explain, and the step rows `STEP_ROWS` keeps) kept its own older guess: half, for
   every filter and anti-join. Two or three value-class bounds cut a rule's rows 4–8×.
2. **A node's kind was independent of the field that bound it.** After `field($i, "object", $n)`, `kind($n, k)` was
   priced at `k`'s share of all the nodes (6% for an attribute), where every such child is of that kind.
3. **A field's lookup by a node of a kind was the field's share of every node.** `field($f, "name", $nm)` for a
   function `$f` was priced at `name`'s postings over all nodes (0.05 a node), where every function has one.

```
attr_of's seed, before (one loop):
  field($n, object, $o)   lookup on (f)     est 2685   made 5301
  $n < 0x6000…            filter            est 1342            <- half, though every row passes
  $o < 0x6000…            filter            est  671
  kind($n, #attribute)    membership        est   55            <- the kind's share of all nodes
  field($n, attr, $at)    lookup on (p, f)  est 5.25   made 5301
after (a loop of 12):
  field($n, attr, $at)    lookup on (f)     est 4685   made 5165
  $at < 0x6000…           filter            est 4591
  field($n, object, $o)   lookup on (p, f)  est 4499   made 5165   [field statistics]
```

## Decision

1. **The record prices a filter as the search did**: `r *= b.filter_pass(s.at)` for every non-equality comparison,
   anti-join and other filter, in the explain and in the step rows. An aggregate is unchanged.
2. **The tree's statistics count fielded edges by kind**: `Own::fields`, per (parent kind, child kind, field), from
   the `field` view's rows of the sampled nodes, scaled as the edges are. They are summed, taken away, kept in the
   memo (`own:fields`) and scaled like the other tree counts. Read from SQL statistics, `stat_edge`'s field column
   gives the same counts. `Stats::fields` (`Fielded`) holds them by (parent kind, field), (child kind, field) and
   all three.
3. **A kind after a field**: `kind($n, k)` with `$n` bound by a placed `field($n, f, _)` passes the share of `f`'s
   edges whose parent is `k`; by `field(_, f, $n)`, whose child is `k`. When the other end's kind is known, the
   share is among that kind's edges by `f`. Several such fields take the least.
4. **A field by a node of a kind**: `field($p, f, $c)` by `$p`, with `$p`'s kind named, yields that kind's children
   by `f` a node; by `$c`, the share of `$c`'s kind that is a child by `f`.
5. **A variable an equality binds to a constant is that constant** where a kind estimate reads it (`$k = #k1207`,
   then `kind($n, $k)`), once the variable is bound. Before it is bound, the scan reads every kind.

## Results

The same run, cold, under the Metal binding, profiled, load 41–55. The steps that make 100 rows or more:

| | rows-weighted mean miss | rows within 2× |
|---|---|---|
| before | 8.2× | 33% |
| after (1–3) | 1.9× | 80% |
| after (1–5) | 2.0× | 84% |

Later pairs, the two builds alternated at load 45–50, gave 5.3× before against 2.8× after: a plan's steps, and
so its misses, differ with each run's waves. The findings are the same in every run: 1,629. Under the CPU binding
the strata's CPU fell from 6.62 s to 4.45 s in one run each (noisy at that load). The plans changed: `attr_of`
now reads its seed from the field with the fewer postings.

A first version read every variable an equality binds as its constant, bound or not. `kind($p, $k)` scans were then
priced at one kind's nodes, where they read every kind (65,000 rows for 5–30 estimated), and the miss grew to
10.5×. Item 5 reads the constant only once the variable is bound.

## What it does not fix

- **Small strata's steps** (rows under a few hundred): `in_first_iter`'s `rr_nearest_lscope` membership, `tp_list`'s
  and `rr_ctx11`'s kind memberships after a `nearest` relation or a derived relation. Their kind correlation is with
  a derived relation, which the tree's counts do not see.
- **`qn`'s `walk($n, $x, $s)` by `(x, s)`**: about 170× under (Q44).
- **The 50-relation `ask_*` stratum**: each probe priced at a delta of 1 row (Q44).

## Tests

- `plan::tests::a_kind_after_a_field_is_priced_by_the_fields_children`: field 7's children are all of kind 5, a
  hundredth of the nodes; the rule is priced at 200 units with the fielded counts, and under 150 without them.
- `plan::corpus::tests::memo_tree_statistics_accumulate_once_without_per_file_maps`: the fielded edges sum, survive
  the memo's rows, and are taken away.
