# 0312: Placement prices a lookup by its relation

Status: built (2026-11-16). Follows 0304 (placement's CPU price counts the executor's unit). `facts/vtab.rs`:
`Source::lookup_cost`; `logic/plan.rs`: `Stats::lookup`; `logic/plan/placement.rs`: `JoinEst::cost`. Work queue Q36.

## Intro

Placement decides, per stratum, whether the CPU or the GPU runs it, from a price on each. Since 0304 the CPU's
price is fitted to the planner's unit of work: 50 ns any unit. That unit is a mix. A lookup into a derived relation
(a hash or sorted index in memory) costs about one. A lookup into some views costs several, because the view
answers it by walking the tree or searching the documents. For example, `within(n, a)` looked up by `n` lists `n`'s
containers one by one.

The profile now gives each join step's CPU, summed by the relation the step reads (`sources.tsv`, Q36). So each
view's lookup can be priced at what it costs.

## Measured

PostHog's `posthog/hogql` (437 files), the light run (`len-_py.rr`), cold, under the CPU binding,
`RERAIL_SQL_STRICT=1`, profiled, at load 31. A step's CPU as a multiple of a derived relation's step:
124 ns a row in (`CPU_PER_PROBE`) plus 72 ns a row out (`CPU_PER_ROW_OUT`).

| relation, looked up by | ns a row in | rows out a row in | multiple |
|---|---|---|---|
| `within` by `n` | 4,143 | 8.8 | 5.5 |
| `docof` by `n` | 816 | 1 | 4 |
| `col` by `n` | 694 | 1 | 3.5 |
| `rr_nearest_*` by `n` | 517–747 | 1 | 3 |
| `child` by `p` | 463 | 1.9 | 2 |
| `field` by `(p, f)` | 315 | 0.9 | 1.7 |
| `text`, `node`, `line` by `n` | 123–211 | 1 | about 1 |
| derived relations | 33–130 | | 0.3–0.7 |

By the other column, the same views cost about one: `within` by `a` and `rr_nearest_scope` by `a` search their
solved pairs.

## Decision

1. **A source says what a lookup on each of its columns costs**: `Source::lookup_cost(column)`, as a multiple of a
   derived relation's lookup, one by default. The views declare the measured multiples above: `Within` by its mode
   and column, and `Viewed` for `docof`, `col`, `child` and `field`. Wrappers (`Renamed`, `Layered`, `Overlay`,
   `Mutable`) pass it on.
2. **The planner's statistics carry it**: `Stats::lookup`, by relation and column name, only where it is not one.
   It is filled where each source is bound with its measured statistics (`plan::corpus`).
3. **Placement prices a join step at its relation's multiple**, by the step's first key column. Its CPU index costs
   the multiple times 124 ns a row in and 72 ns a row out. The stratum's CPU price adds what the step costs past one,
   since the units of planned work are already priced at about one.

The multiples are ratios, so they hold as the machine's load changes the times. They are fitted like 0304's prices,
and are fitted again the same way: from `sources.tsv`.

## Results

Under the Metal binding, the same run before and after, gave 1,629 findings each. The price reaches placement:
`qualname`'s seed looks up `rr_nearest_scope` by `n` on about 10,000 rows, and its price rose by the 4 ms the
multiple gives. No stratum changed place. The costliest view lookups (`within` and `rr_nearest_*` by `n`) are in
strata the GPU already runs (`pre_119`, `wstart`, `walk`), and the CPU's strata read views at one to two.

## What it does not fix

The strata still priced far under their CPU miss by their rows, not by their lookups:

- **Their estimates.** In `attr_of`, `rdefn`, `ann_root` and `qualname`, a step is estimated at 2.5–46 rows and makes
  450–870,000. Each `$n < 6917529027641081856` filter (a value-class bound, `0x6000…`, that every node passes) is
  estimated to keep half its rows. Two or three in a rule cut its estimate 4–8×. `kind($n, #k)` membership after a
  `field` lookup is also given a few percent where every row passes. These are the planner's statistics (Q44).
- **A lookup whose cost no per-relation price holds.** `fzk287`'s `col($v, $c)` by `n` ran 790 rows over 17 chunks
  in 52 ms: 65 µs a row, against `col`'s 694 ns over the whole run. The line index it reads is built during
  projection, at about 10 µs a file, so the index is not it. The cost looks fixed per chunk, not per row; a sample
  of that step will say where it goes (Q36).

## Tests

`placement::tests::a_stratum_is_placed_by_its_price_and_named_by_the_forms_the_device_lacks`: a step on a view whose
lookup costs 5.5 is priced 5.5 times a derived relation's step on its index, and adds 4.5 times that to the
stratum's CPU; a cost of one leaves the price as the units give it.
