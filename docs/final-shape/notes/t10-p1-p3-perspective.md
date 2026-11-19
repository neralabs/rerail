# T10 notes: a perspective on span containment (P1) and file-level existentials (P3) (2026-11-06)

## Intro

Two rule shapes cause most of the remaining blow-ups. Each one makes a rule's cost grow
with the size of a file, not with the number of matches:

- **P1, span containment written as comparisons.** "Node B's span contains node A's span"
  is written as two `node` atoms on the same file and two `<=` tests. The engine pairs
  every node of the file with every other node, then filters. The cost is the file's
  nodes squared.
- **P3, a part that only says "the file contains X".** One part of a rule body is joined
  to the rest only by the file. The engine enumerates every combination of the two
  parts, so the cost is the file's matches times the rest's rows.

This note sets out where each shape comes from, what the exact semantics are, the fix
I recommend, and what it should buy. It proposes; nothing here is implemented yet.

How common they are, across the 100 converted rules:

| Shape | Rules | Uses | Worst measured |
|---|---|---|---|
| P1 | 8 | 14 clauses | transaction-closures: 3.1 billion rows on 200 files (71 s) |
| P3 | 8 | about 106 clauses (a loose count) | module-level-caches: 500M rows on 50 files (38 s) |

Neither of these two rules can run on the full 9,010-file corpus today.

## 1. P1: span containment

### Where it comes from

The converter writes "inside" as span comparisons when it doesn't know the container's
kind. The helper is `within` in `native/src/code_rule_dsl/logic/convert.rs`, and it is
used in three places: `has` from a node of unknown kind, field paths, and the "blocked"
relation of a has-stop. A sample of what it emits:

```
// "b contains a": both on file d, with b's span around a's.
node(d, a, _, s, t), node(d, b, _, u, w), u <= s, t <= w
```

The planner sees two scans of `node` joined only by `d`, followed by two filters. Nothing
in the relational form says the comparisons describe a tree.

### Exact semantics

Spans in a syntax tree form a laminar family: any two spans are either nested or
disjoint. So for a node A, the nodes whose span contains A's span are:

1. A's ancestors (all of them contain A's span);
2. A itself;
3. descendants of A with the same span as A (a wrapper with a single child, for example);
4. zero-width nodes at the edges: a zero-width node at A's start or end position has a
   span contained in A's, and a zero-width A is contained by any node that touches its
   position, including a sibling that ends where A starts.

The opposite direction, the nodes contained in B, is B's subtree (a preorder interval)
plus the same edge cases.

A rewrite must keep every one of these. The recorded findings are the test: the replay
must stay identical.

### What the prior work says

- The spike settled on "preorder plus subtree size (a nested interval). Descendant test
  is an O(1) range check" (design index, D S012).
- It recommended exposing extents and letting the caller write
  `pre BETWEEN anc.pre AND anc.pre + anc.size`, but noted that "the planner behaviour of
  that join is unmeasured" and asked for that measurement (design index, LCS G1).
  P1 is that measurement: with spans in place of preorder, the planner's behaviour is
  quadratic.
- "Arbitrary-depth descendant is an interval test on preorder plus size; there is no need
  for recursion" (design index).
- The history index's recurring illness: "a per-file cost turned into a per-row cost …
  Look for these first when a query scales with candidates times file size" (history
  index §6).

### Options

1. **Converter only: emit `inside` where kinds are known.** This is cheap and it removes
   some uses. It doesn't remove the `has` from an unknown kind, and decision 0048's point
   stands: conversions are imperfect, and the engine must not depend on them.
2. **Engine: a range join on spans.** Sort the file's nodes by start position and probe
   with a range. This is generic, but containment needs two ranges at once (start ≤ s and
   end ≥ t), so it is still one dimension short. It is better, but it is not linear.
3. **Engine: recognize the idiom and lower it to a structural relation (recommended).**
   The planner matches the pattern: two `node` atoms on the same file variable, plus the
   two comparisons over their span columns. It replaces them with a native relation
   `span_within(d, a, b)`, read off the tape, with two access directions:
   - `a` bound: walk A's ancestor chain, then add the same-span descendants and the
     zero-width edge cases. This costs about the tree's depth.
   - `b` bound: B's preorder interval, plus the edge cases. This costs B's subtree.

   Its estimate comes from statistics the tape already has: mean depth, and mean subtree
   size per kind.

### Expected effect

transaction-closures goes from 3.1 billion rows on 200 files to about nodes × depth:
530k × about 20, so roughly 10M rows on s200, and about 0.6 billion on the full corpus (A).
m650 in serverplan-quality goes from 13.3M to about 1M rows (A). The rule becomes
runnable on the full corpus.

## 2. P3: a part that only says "the file contains X"

### Where it comes from

A sample, from module-level-caches:

```
// The violation: some node n inside a module, in a file that names X somewhere.
violation(n) :-
    strtest_6(s), name(y, s), node(d, y, _),   // "file d contains the name X"
    node(d, m, 1132 /* module */),             // the file's module node
    inside_k0(n, m), ...                       // n inside it, and the rest of the rule
```

The first line shares only `d` with the rest. It is a yes/no fact about the file, but the
join carries `y` along: every name in the file that passes the test multiplies the rest's
rows. A file that names X 50 times does the rest's work 50 times, and then the results
are made distinct again.

### Exact semantics

Under set semantics, a part of a body whose variables meet the rest only at `d`, and none
of whose other variables appear in the head, can be replaced by its projection onto `d`:

```
aux_6(d) :- strtest_6(s), name(y, s), node(d, y, _).
violation(n) :- aux_6(d), node(d, m, 1132), inside_k0(n, m), ...
```

The result is the same set of findings. The same holds for an interface of more than one
variable: the projection keeps exactly the variables the rest of the body (or the head)
uses. Negated atoms are already separate relations, so only positive parts are
projected.

### What the prior work says

- The spike's branch-collapse ladder: "capture-free children: EXISTS semijoin (131 to
  67 ms)"; "bottom-up subtree collapse (per-level DISTINCT MATERIALIZED): 25 s abort to
  754 ms" (design index, D S052/S054).
- S055 measured a "redundant candidate-file semijoin": a semi-join on the file is worth
  it when it removes a factor, not when it only repeats a filter.
- This is Yannakakis's semi-join reduction, restricted to the cheap case: a component
  joined through a small interface.

### Recommendation

1. **A simplifier pass, in decision 0038's family.** Split each body into connected
   components by shared variables, ignoring the file variable. Any component that has no
   head variables and meets the rest only at its interface becomes an auxiliary relation
   projected onto the interface. It is a rewrite of the rule, before planning, so it
   applies to every rule and every backend.
2. **Then fix the two planner estimates that P3 exposed:**
   - descending from an ancestor (`inside_k1` with a module ancestor) is estimated far too
     cheaply;
   - a text test that is empty in the planning chunk gets an estimate of zero. Flooring
     it at 1 was tried and reverted (773e9 instructions instead of 415e9): with the floor,
     the planner started from "the file contains name X" and walked down from the module
     node. With the projection in place, that start becomes cheap, so the floor should be
     tried again after step 1, not before.

### Expected effect

module-level-caches goes from 500M rows on 50 files to about the rest of the rule's rows
plus one row per matching file (A). error-handling's m17 (18.9M rows) should drop the
same way. The 4 serverplan-quality violations of this shape (3.3–7.3M rows each) should
drop to about the matches.

## 3. Order of work

1. P3's projection pass first. It is a simplifier rule, small and testable (`logic::`
   tests plus the replay), and it covers more rules (8 rules, about 106 clauses).
2. P1's structural relation second. It needs a new native relation over the tape, its
   two access paths, and its estimates, so it is larger. The edge cases (same-span
   descendants, zero-width nodes) need their own tests against the comparison form.
3. The converter's `inside` where kinds are known, last, and only as tidying. The engine
   handles both forms by then.

   Done (decision 0050 §4). No converted clause knew the ancestor's kinds, so none could use
   `inside`. The converter writes the language's `within` instead, which the engine serves
   from the same tree relation.

Both fixes are measured the same way: the replay totals stay identical, and instructions
retired go down on s200 and on the full corpus.
