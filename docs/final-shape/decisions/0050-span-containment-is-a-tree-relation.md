# 0050: Containment is a relation read off the tree: `within(n, a)` and `span_within(inner, outer)`

Status: accepted (2026-11-06; built and measured; amended the same day: the language's `within` is served by the
same relation, and the converter writes it, §4)

## Intro

Rules often ask whether one node lies inside another. When the converter doesn't know the outer
node's kind, it writes the question as two span comparisons:

```
// "b holds a": two nodes of file d, b's span around a's.
node(d, a, _, s, t), node(d, b, _, u, w), u <= s, t <= w
```

To the planner, this is two scans of `node` joined only on the file, then two filters. It pairs
every node of a file with every other node, so the cost is quadratic in file size. On 200 files,
transaction-closures produced 3.1 billion rows (71 s). serverplan-quality's `m650` produced 13.3
million on the full corpus (notes `t10-p1-p3-perspective.md`, P1).

## Decision

### 1. The simplifier makes the comparisons a relation

`simplify::span_containment` looks for two positive `node` atoms on the same file variable, with
`u <= s` and `t <= w` between their spans (written either way round). It replaces the two
comparisons with `span_within(a, b)`. The `node` atoms stay, because they may bind or test more. The
span variables then usually become `_`. `span_within(inner: node, outer: node)` is declared as an
input.

### 2. The relation is the tree's, made on lookup

Syntax-tree spans are laminar: two spans either nest or are disjoint. `syntax::within::Within`
uses that to answer both directions exactly:

- **The containers of a node** (`inner` bound) are:
  - the node itself;
  - its ancestors;
  - its descendants with the same span (a wrapper with one child).

  A zero-width node is held by every node that touches its position, siblings included. For one,
  the file's nodes are read.
- **The nodes a node holds** (`outer` bound) are:
  - its subtree (a preorder interval);
  - its ancestors with the same span;
  - the file's zero-width nodes inside its span.

Rows are made only for the nodes a join looks up. They are kept for the batch in chunks that never
move, so a row's id is stable and reading it takes no lock. The two directions keep their rows
apart: rows made from `inner` have ids from 0, rows made from `outer` ids from 2^31. A scan reads
ids `0..len` after making every node's containers, so it sees each pair once. (The first build kept
one store, so a scan after lookups by `outer` saw some pairs twice; the brute-force test now looks
up by `outer` before it scans.) A scan makes every node's containers,
about the sum of (depth + 1) rows, which is linear in the nodes times their depth.

The planner's statistics come from the tree, not from the rows:

- rows: the sum of (depth + 1);
- per inner node: depth + 1 rows;
- per outer node: its subtree size.

That needs a `Source::statistics` hook. Loading a whole view no longer counts its rows up front
either: `InRel::len` asks the source only when a step scans.

### 3. Tests

- `within_holds_exactly_the_nested_pairs` checks the relation against comparing every pair of
  nodes. It looks up both columns and scans. It uses three corpus files plus four files with
  errors or empty bodies, which bring zero-width nodes.
- `two_nodes_of_a_document_compared_by_span_are_within` checks the rewrite, and that two nodes on
  different file variables are left alone.

### 4. The language's `within` is the same relation, in tree mode

`core.rr` already had `within(n, a)`: n strictly below a. It was lowered at read time to a range
over preorder ids (`tape(a, _, z, _), n > a, n <= a + z`), or, when nothing else bound n, to a
recursive `below` relation climbing `child`. That recursion is what `core.rr` says the tape exists
to avoid.

Now the lowering keeps `within(n, a)` as an atom, and the engine serves it from the same source in
tree mode:

- `n` bound: its ancestors (its depth);
- `a` bound: its subtree, the ids `a + 1 ..= a + size` (its size).

`within_stop` is that atom plus the stop test, as before. The span relation is named `span_within`,
so one name has one meaning.

The converter now writes `within(m, n)` for `has` from a node of unknown kinds and for its
`blocked` relation (13 of the 14 clauses that compared spans). Before, it wrote span comparisons.
The 14th, a `has` through a field, says "the descendant is the field's child or below it". The
front syntax the converter writes has no alternatives, and the child is one of its parent's
children, whose spans don't overlap. So it keeps the span comparisons, and the engine makes them
`span_within`.

Where the ancestor's kinds are known, the converter already writes `inside_<set>`. None of the 14
clauses knew them: in each, only the inner node's kind was fixed (`call`, `identifier`).

## What the prior work says

- "Shape: preorder plus subtree size (a nested interval). Descendant test is an O(1) range check"
  (design index, D S012 L313-332). The `outer` direction is that interval. The `inner` direction is
  the ancestor walk the tape already recovers.
- "Arbitrary-depth descendant is an interval test on preorder plus size; there is no need for
  recursion" (design index, D S012; LCS G1).
- The spike left "the planner behaviour of that join" unmeasured (design index, LCS G1). This
  record measures it: written as span comparisons, the join is quadratic per file, so the engine
  gives it the tree's access paths instead.
- The history index's recurring illness is "a per-file cost turned into a per-row cost" (history
  index §6). The pairwise span join was that illness.

## Measured (2026-11-06)

Instructions retired, with machine load 50–97:

| Run | Before | Now |
|---|---|---|
| transaction-closures, 200 files | 71.3 s CPU | 0.46 s CPU (4.9e9) |
| transaction-closures, 9,010 files | did not finish in the run cap | 235.4e9 (22.2 s CPU) |
| serverplan-quality, 9,010 files | 371.2e9 | 355.6e9 |
| `m650`'s join, rows into its steps (9,010 files) | 13.3 million | about 24 thousand |

The replay's totals are unchanged (3988 273 228 0, 1638 95 55 0, 2350 178 173 0), with both span
and tree semantics for the 13 strict clauses.

After §4: transaction-closures 4.9e9 (200 files) and 236.4e9 (9,010 files); serverplan-quality
355.1e9 and 361.4e9 in two runs at load 67–109. The joins' rows (14,394,225) and SQL VM steps were
identical in both profiles, so the spread is load. `m650` now reads `within(call, n)` by `n`
(986 → 9,217 rows), with no `!=` filter after it.

The first build counted a view's rows when loading it. That made all of `within` at engine open,
and serverplan-quality rose to 413e9. Counting on first scan fixed it.
