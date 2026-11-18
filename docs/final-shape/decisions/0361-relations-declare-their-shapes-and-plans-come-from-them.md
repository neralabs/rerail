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

## Step 7: a count below a bound, by form (2026-11-17, Q164)

**No rewrite is keyed on a relation's name** (the user, 2026-11-17). A first form (Q163) took `count : { child(g, y,
_), F(y), y < x }` by `child`'s name and its dense index, and lowered it to a running count. It is removed. It
stopped applying when a rule was written another way, and it relied on facts the program never states.

`logic::rank_form` finds the form by the rules' structure alone. A reader `n = count : H(…)` qualifies when:
- `H`'s one rule holds a comparison `ky OP kx` (`<`, `<=`, `>`, `>=`);
- without it, `H`'s body falls in two parts by the variables its literals share, the grouped ones aside:
  - the **counted part** `Y`: the literals tied to the counted columns (the reader's `_`) and to `ky`;
  - the **asked part** `X`: the rest, which binds every grouped column and `kx`.

The **group** is the grouped variables `Y` reads. The two parts are then relations of their own, and a native
operator counts:

```
rr_rankq(G…, A…, kx) :- X.                 the asked rows, each with its bound
rr_rankg(G…) :- rr_rankq(G…, _…, _).       their groups
rr_rankb(G…, C…, ky) :- rr_rankg(G…), Y.   the counted rows of those groups
rr_rank(G…, A…, n)                          Native::Rank; the reader's count becomes `n = sum m : rr_rank(…, m)`
```

`Native::Rank` (`logic::native::rank`) sorts each group's keys once and answers each asked tuple by binary search.
That is O((Q + C) log C) work, where `H` held Q × C pairs in each group. The count stays exact:
- a counted tuple of several keys counts by its least key (for `<` and `<=`) or its greatest (for `>` and `>=`);
- an asked tuple of several bounds is counted at its greatest bound (for `<` and `<=`) or its least.

The sum is 0 where nothing is asked, as the count was.

The rank's kept rule joins its two relations on the group, as `H` did. So the file-local split judges it as it judges
any rule: it is local where its groups lie in one document. It is placed after demand (magic sets), and the
relations it replaced are pruned. `RERAIL_RANK_FORM=off` finds none, a measurement aid, and `.decl H(…) no_rank`
keeps `H` as rows.

Measured at 300 files (cold58 against cold56, load 70–110):

| | CPU | Work |
|---|---|---|
| The aggregate helpers (`rr_agg*`), before | 2.06 s | 20.7 M |
| The aggregate helpers, after | 0.44 s | 3.6 M |
| The rank's relations | ≈0.5 s | |
| Every head together, before | | 437.8 M |
| Every head together, after | | 413.9 M |

- **Where it applied:** 30 counts across the program, python/ast's `body_stmt` among them.
- **Findings:** the reference's.
- **Peak:** 2.29 GB.
- **Strict 10-file:** outputs byte-identical.

When `.layout` lands (steps 1–2), a declared `order(i dense in p)` may let the engine choose a running walk over
the sort. The form itself needs no declared fact.
