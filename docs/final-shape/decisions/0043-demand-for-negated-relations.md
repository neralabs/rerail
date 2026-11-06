# 0043: Relations that rules only negate are computed for the keys the rules reach

Status: accepted (2026-11-06)

## Intro

An imperfect conversion can leave a relation that is computed for every node of the corpus, although
the rules that read it ask about only a few. The case that prompted this record is SPQ-044's
`blocked1243`. The converter spelled a `has` test as an ancestor walk from every node:

```text
// One row for every (node, enclosing node) pair the walk reaches: 1.8M rows on 1000 files.
blocked1243($low, $high) :- nearest_k11($low, $s), inside_k2($s, $high).

// The one reader: two negations of it, for two nodes the rest of the body binds.
violation(…) :- <27 atoms binding $n, $v1242, $v1250, …>,
                !blocked1243($v1242, $n), !blocked1243($v1250, $n).
```

The converter can be fixed for this pattern later (a `has` from a node whose kinds lie in the stop
kinds is a `nearest`). But conversions will stay imperfect (user, 2026-11-06: "I want to improve the
ability to execute that, so that we can handle imperfect conversions"). So the engine must not
pay for every node when a rule only negates the relation.

## Decision

In a corpus run (the run that keeps plans), before stratification, `plan::demand::split_negations`
rewrites each **demandable** relation `R` and each rule that negates it:

```text
pre_<i>(vars)      :- <the reader's body without its negations of demanded relations>.
demand_R(args)     :- pre_<i>(vars).              // one per negation of R in the reader
R(k, …)            :- demand_R(k, …), <R's body>.  // every rule of R
reader-head        :- pre_<i>(vars), !R(args), …. // the negations, now over pre_<i>
```

Here `vars` are the variables the body binds that the head or the negations use.

A relation is **demandable** when it is derived and not kept (output, input, lattice, fact, demand,
gate), not recursive, and every one of its rules starts from every node: a `nearest_<set>`,
`inside_<set>` or kind-free `node` atom on its key. It must also be read, and every read must be a
negation whose arguments are variables or constants. If a variable has no known type, or the result
does not stratify, nothing is rewritten. The profile then gives the reason under
"demand (decision 0043)".

It complements the earlier key-only demand of `magic::demand_negated_tests` (m1028, m1597, m819).
That rewrite seeds a per-node test from the reader's atoms over its key alone. This one seeds a
relation from the reader's whole body.

### Why this shape

- **Set at a time, keyed anti-join.** The negation stays a keyed anti-join over a materialized
  `pre_<i>`, never a probe per outer row. In the spike, an inlined correlated reference re-ran the
  body per row: `json_each` ran for 8+ minutes, and S203 went from 296 to 31 ms when the CTE was
  materialized (history index L62; design index L796, L811, S064). A keyed anti-join beat a
  SubPlan rescan, 1,214 to 98 ms (history index L585, S254). Dominance uses a keyed anti-join only
  with invariant keys (design index L616, S271/S272). Here the keys are the negation's own
  arguments.
- **A structural guard, not statistics.** The spike's lesson is that "any SQL backend with a
  cost-based planner will misjudge recursive or vtab cardinalities. Plan a structural guard, not
  statistics" (history index L59). `pre_<i>` is the reader's own body less some anti-joins, so the
  demand cannot exceed what the reader binds anyway. The negations prune only at the end, which is
  cheap for these relations.
- **No plan frozen into the rewrite.** `pre_<i>` is planned freely in every chunk like any rule, and
  no copies or cycles are needed. Every demand of a relation negated twice comes from the one
  `pre_<i>`.

### Rejected

Measured on s1000 (1000 files, 3.1M nodes):

| Variant | What happened |
|---|---|
| magic-sets demand from the reader's whole positive prefix (`magic_sets`) | 225M rows, 74 s |
| split the reader where its plan meets the negation, plan taken at setup | the prefix ran to 20M rows: estimates for derived relations are defaults at setup |
| the same, chained through one rule per negation | two negations of `blocked1243` in one rule closed a stratification cycle (fixed with copies, then dropped) |
| the same, plan frozen from chunk 1's live plan | `pre_180_0` 320M rows, 54.6 s CPU: chunk 1's path test matched nothing, so the cut came before the selective identifier test |

## Results

On s1000, serverplan_quality, with CPU = user + sys at load 32:

| | before | after |
|---|---|---|
| run | 7.14 s | 3.95 s |
| `local` phase | 2.36 s | 1.22 s |
| `blocked1243` rows | 1.8M | 0 (demanded keys only) |

- **Rewrite reach:** `blocked1243` is demanded in all 12 chunks.
- **File-local split:** 269 rules are file-local and 0 are cross-file.
- **Checks:** the corpus replay is identical. The logic tests pass, apart from the existing failure
  of `data_layer_census_matches_the_rules`.

## Follow-ups

- Converter: a `has` from a node whose kinds lie in the stop kinds converts to `nearest`. This is
  deferred.
