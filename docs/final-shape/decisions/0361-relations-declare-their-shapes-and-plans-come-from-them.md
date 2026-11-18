# 0361. Relations declare their shapes, and plans come from them

Status: decided (2026-11-17), to build. The user: "a directive which allows us to add enough data to the
relations in the .rr files that we can provide good default shapes for them (or at least name an algorithm or
something) so that we don't need to pay for planning, or only in exceptional circumstances."

## Intro

Today the planner learns each relation's shape at run time. It counts statistics (distinct values, squares of
counts, heavy values, exact pair statistics), prices orders from them, and searches. At 300 files this is a visible
share of a cold run: planning strata 3.2 %, live statistics and bookkeeping 1.8 %, exact pair statistics about 0.2 s
in each stratum that asks for them, and the join-order search (cold13).

Most of what it learns is known when the rules are written. Some examples:

- A node has one parent, so `child(p, c, i)` has at most one row for each `c`.
- A node's children are numbered 0, 1, 2, … in order, so `i` is dense within each `p`.
- A node has one kind, and every node lies in one document.

When the planner has to rediscover these, it can get them wrong. `rr_agg56` pairs every child with every earlier
sibling, 80 K rows to 1.6 M, because nothing tells the planner that "earlier sibling" over a dense order is a running
scan.

The idea: the `.rr` files state these facts once, mostly in `core.rr` for the base relations. The engine infers the
same facts for derived relations. Plans are made from the facts when the program is prepared, kept in its image, and
not searched again at run time. Run time only checks that the facts still hold. Planning from statistics remains for
the exceptions: a relation with no declared or inferred shape, or one whose rows broke what it declared.

## What a relation can declare

Each item is a fact about the rows, not about a plan. So it holds for every form made from the relation's rules:
deltas, maintenance forms (`__ins`, `__del`, `__redo`, `__supN`), and the hiding copies. 0137 found that a fixed
order would be wrong for most of these.

| Fact | Example | What the planner gets from it |
|---|---|---|
| **Key** (a functional dependency) | `child(p, c, i)`: key `c`, key `(p, i)` | A probe on the key returns at most one row. Joins on a key don't multiply rows. A projection that keeps the key needs no dedup (0329 is the special case for `node`). |
| **Fan-out class** | `child`: `p -> c` few | The rows a probe returns, without counting: one, few (16 or fewer), or many. |
| **Size** | `node`: one row per node. `kind`: one row per node. `language_table`: small | Relation sizes from counts the batch already has exactly (nodes, documents, per-kind counts, `Batch::kinds`). |
| **Local** | every column of `child` is a node of one document | Per-document joins: no rows across documents, and work partitions by document (chunks, the device). |
| **Order** | `child`: `i` is dense from 0 within each `p` | "Earlier" or "later" sibling becomes a walk along `i`. An aggregate over earlier siblings becomes a running aggregate: `rr_agg56` without the 1.6 M pairs. |
| **Tree** | `child`: `p -> c` is a forest | Reach over it is a walk up or down, with its depth bounded by the tree's. |
| **Algorithm** | `chain_below`: by reach. `owner_code`: by earlier (exists: `ordered by earlier`) | Names the native operator that makes the relation, where the rule's form alone doesn't show it. |

## Inferred for derived relations

Declarations go where inference can't reach: base relations, aggregates, and some recursion. Every other relation
gets its facts by propagating them through its rules, as types are propagated:

- A projection keeps a key if it keeps the key's columns.
- A join along a key keeps the other side's key and fan-out.
- A filter keeps everything, and can shrink the size class.
- A union keeps a fact only if every branch has it.
- `local` holds if every literal is local and they share a document variable.

Recursion keeps a fact only when it is declared, or proved by the base case and the step.

## Plans from shapes

- When the program is prepared, each rule form is planned once from the facts. Estimates come from keys and fan-out
  classes, sizes from the batch's exact counts. The plans go into the program's image (0246) with the rest of what
  preparing makes, keyed by the facts. A run reads them and does not plan.
- No statistics are gathered for a relation whose shape is declared or inferred.
- **The exceptions.** A relation with no shape is planned from statistics, as today. A relation whose rows break what
  it declares is replanned from statistics, and the profile names it. The executors already count every step's
  rows (0203), so a fan-out class broken by large factors is seen with no extra pass. A broken key is seen where the
  rows are written: dedup already probes the whole row, and a key needs one more probe.
- 0137's `.shape B after A` and `.shape B bound x` stay as the per-rule escape hatch. The test that tells you to
  remove an unneeded directive then also covers declared facts the planner no longer needs.

## Decided with the user (2026-11-17)

1. **A line per relation, `.layout`.** `.decl` stays as Soufflé reads it:

   ```
   .decl child(p: node, c: node, i: number)
   .layout child key(c) key(p, i)
           fanout(p -> c, few)
           order(i dense in p) tree(p -> c) local
   ```

   `.layout` names a declared relation. A relation may have several `.layout` lines, which add up. A fact naming a
   column the relation lacks is a syntax error with the line. `combine` carries the lines with the program and
   renames them as it renames relations, as it does 0137's `.shape`.
2. **Classes, numbers optional.** Fan-out is `one`, `few` (16 or fewer) or `many`. Size is `per n` (at most one row
   per value of column `n`), `per document`, `per kind` or `small`. A measured number may be given instead, for
   example `fanout(p -> c, ~4)` or `size(<= 2 per n)`. Classes are what plans are keyed by, so a number that changes
   does not invalidate a kept plan.
3. **A broken fact replans and is reported.** The relation is planned from statistics for the rest of the run, and
   the profile names the relation and the fact (a strict run's report too). The run goes on.
4. **Algorithms are found from the facts.** Two examples:
   - order plus an aggregate over the earlier rows of a group gives a running aggregate;
   - a tree plus recursion along it gives a walk.

   An explicit override is allowed for rare cases: `.layout r by reach`, `by walk(child)`, `by running(i in p)`.
   Like 0137's directives, a profiled run says when the override is no longer needed.

## Build steps

1. The `.layout` line: parse, check, carry through lowering, `combine`, the image and the plan-choices digest.
2. `core.rr`'s base relations declared: `node`, `kind`, `child`, `field`, `node_order`, `docof`, `text`, `name`.
3. Shape inference over the rules, as above, kept with the program's version; EXPLAIN shows each relation's facts and
   where they came from (declared or which rule).
4. The planner prices from facts: key and fan-out estimates, sizes from the batch's counts; no statistics requested
   for a relation with a shape.
5. Plans made at preparation and kept in the image; a run plans none of the relations that have shapes.
6. The checks: keys where the rows are written, fan-out from the executors' counts (0203); a broken fact replans
   and is reported.
7. Algorithms from the facts: the running aggregate along an order first. This is the `rr_agg56` planner fix: 80 K
   children, no 1.6 M sibling pairs.

Done when a cold 300-file run plans nothing at run time outside the exceptions, its findings are the reference's,
and the planning share of its profile (cold13: 3.2 % plus statistics) is gone.

## Step 7, first form: the running count (2026-11-17, Q88)

Built ahead of steps 1–6, by the form of the rule, with `child`'s order as core.rr states it today: a node's
children are numbered from 0, densely, in node order. When `.layout` lands (step 2), the same rewrite is to be
keyed by `order(i dense in p)` and not by the relation's name.

Lowering (`rr::lower`, `running_count`) takes an aggregate `i = count : { child(g, y, _), F(y), y < x }` (or
`x > y`) in a rule that also holds `child(g, x, _)`. `F` must read only `y` and `g`. It writes the aggregate as a
running count along the index:

```
rr_runf(g, y, i) :- ctx(g), child(g, y, i), F(y).
rr_run(g, 0, 0) :- ctx(g).
rr_run(g, j, k1) :- rr_run(g, i, k), rr_runf(g, y, i), j = i + 1, child(g, _, j), k1 = k + 1, k1 <= j.
rr_run(g, j, k)  :- rr_run(g, i, k), child(g, y, i), !rr_runf(g, y, i), j = i + 1, child(g, _, j).
// the aggregate:   child(g, x, ix), rr_run(g, ix, i)
```

These are ordinary rules on the engine's own fixpoint, with no new evaluator:
- Each step reads the next child, which bounds the index the way 0043's joins in child order are bounded.
- `k1 <= j` always holds, and it bounds the count for `front::cycles`.
- Maintenance is the fixpoint's, as for any recursion.

Measured at 300 files (cold57 against cold56), python/ast's `body_stmt`:

| | CPU | Work |
|---|---|---|
| `rr_agg56` (the sibling pairs) with `body_stmt` | 2.36 s | 17.1 M |
| `rr_run` with `rr_runf` and `body_stmt` | 0.6 s | 2.2 M |

The other results:
- `rr_agg56`'s fan-out (80 K to 1.6 M rows) is gone from the analysis.
- Findings are the reference's.
- The peak was 2.35 GB.
- Strict 10-file outputs are byte-identical.

lengths.rr's `len_splat` (`kind(s2, "list_splat")`) has the same form and takes the rewrite too.
