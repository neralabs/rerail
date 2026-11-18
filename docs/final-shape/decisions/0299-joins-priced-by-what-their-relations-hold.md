# 0299: Joins priced by what their relations hold

Status: built (2026-11-16). Follows 0297 (an order's price is the executor's count). `logic/plan.rs`: `atom_rows_`,
`binding_domains`, `Stats::subtree_fan`, `make_plan` (sizing a relation of facts).

## Intro

0297 made the planner's price count what the executor counts, and left three joins whose price was off by their
statistics, not their filters. Each read a statistic that did not describe what the join reads:

| rule | priced | ran | the statistic read |
|---|---|---|---|
| `pth_deferred` (`within($f, $s)` with `$s` bound) | 0.24 M | 30 M (one run) | `within`'s fanout from its container, measured over the lookups made so far |
| `region` (`comp_kind($ck, $k), kind($c, $ck)`) | 10 M | 0.17 M | `comp_kind` unsized (1,000 rows by default), and `kind` by a bound kind priced at every node over 88 kinds |
| `annotated_on_chain` (`demand_annotated_on_chain($n, $x), walk($n, $x, $s)`) | 0.03 M | 6.8 M | `walk`'s two bound columns taken as independent |

```
// The container's descendants, not a lookup's: `within($f, $s)` with `$s` bound reads $s's subtree.
pth_deferred($n, $s) :- demand_pth_deferred($n, $s), look($n, _, $s), rr_nearest_ks4($n, $f), within($f, $s).
// Four comprehension kinds, written as facts: `kind(c, ck)` reads those kinds' nodes, a few thousand.
region(r, c, k) :- comp_kind(ck, k), kind(c, ck), child(c, r, _), !kind(r, "comment"), !kind(r, "for_in_clause").
```

## Decision

1. **`within` read from its container** (`within($n, $a)` with `$a` bound, `$n` not) yields the container's
   subtree: the `size` statistic of `$a`'s kind when the body names it, else `Stats::subtree_fan`, the mean subtree
   size of the kinds that have children (`size >= 2`), weighted by how many nodes each kind has.
2. **A relation of facts alone is sized when planned.** A relation that only the program's facts fill (no rule
   derives it, no input) gets its row count, each column's distinct values, fanout (Σc²/n) and value counts
   (`dfreq`) from the facts, before it is made.
3. **`kind($c, $k)` with `$k` bound by an atom whose values there are counted** (`dfreq`: facts, or a derived
   relation of few values) reads those kinds' nodes: the mean of each kind's node count, weighted by how often the
   value occurs. The same set caps `$c`'s domain (`binding_domains`), so a probe by `$c` into a relation of scopes
   is not priced as if most scopes missed.
4. **A probe on several bound columns, none constant, reads at least one combination's rows**: the relation's rows
   over the product of the columns' distinct counts, capped at its rows. Independence multiplied each column's
   selectivity: `walk($n, $x, $s)` with `$n` and `$x` bound was priced as if `$x` narrowed `$n`'s rows again,
   though `$x` is `$n`'s name and narrows nothing.

## Measured

300 files × 317 rules (`rerail-shared/bench/step-vs-main`), cold, `RERAIL_JOIN_SEED=1`, `RERAIL_JOIN_LINES`. All
output relations have the same row counts as with 0297.

| | 0297 | this |
|---|---|---|
| work ÷ price, whole run | 1.01 | 1.04 |
| work priced within 2× | 92% | 93% |
| join work, whole run | 545.5 M | 547.6 M |
| `annotated_on_chain` work / price | 7.31 / 0.04 M | 0.07 / 0.09 M |
| `region` work / price | 0.27 / 10.07 M | 0.13 / 0.14 M |
| `pth_deferred` work / price | 0.26 / 0.26 M | 0.26 / 0.26 M (two runs) |

The largest moves between the two runs are lines whose order is the same in both (`rr_agg57`, the `pre_12x`
demand lines, renumbered from run to run); their work follows the demand each run brings.

## Tried and dropped

- **`within`'s subtree weighted by size.** Weighting each kind's subtree by its size makes the mean the size of the
  subtree a random node sits in, near a module's. `annotation($n) :- ann_root($t), within($n, $t)` then priced the
  container side above a scan of `within`, and ran 37 M rows against 0.28 M; `ann_ctx` the same.
- **Pricing `kind($c, $k)` from the bound kinds without capping `$c`'s domain.** `tscope`'s
  `rr_nearest_ks10($t, $s), kind($s, $k), scope($k)` took `scope($k), kind($s, $k)` first, 1.7 M rows against
  0.07 M: `$s` still ranged over every node, so the probe of `rr_nearest_ks10` by `$s` looked like it mostly missed.

## Tests

`plan::tests`:
- `within_read_from_its_container_is_priced_by_subtree_size`
- `a_relation_of_facts_is_sized_when_planned`
- `a_kind_bound_by_counted_values_reads_their_nodes`
- `a_probe_on_several_columns_reads_a_combinations_rows`
