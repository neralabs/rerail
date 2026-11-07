# 0059: A round reruns only the documents whose inputs changed

Status: proposed (2026-11-07). Step 1 built and measured: a document is too coarse; the unit is revised to the node (6)

## Intro

A typing program's cross-file part runs in bounded rounds (0035, 0047, 0056). Round r reads round r − 1's rows of
the hop relations (`<rel>__prev`) wherever a chain crosses a file. It stops when the digest of all hop rows is
unchanged, or at k = 4.

Every round runs the whole cross-file part again, over every document. Most documents' rows stop changing after
the first rounds:

```text
// documents whose hop rows changed since the round before, 196 documents (an earlier profile)
round      0     1     2     3     4     5
changed  195   194   126   148    39    46
```

That is why k cannot be raised. A chain through a library needs more crossings than four. For example,
`next(iter(xs))` on a `list[V]` needs eight with typeshed: `iter`'s overloads, `list.__iter__`, `Iterator`,
`SupportsNext` and `Iterator.__next__` lie in four documents. Each extra round costs a full round, and the rows it
adds are few.

03 §13 names the remedy: rounds "with an early cutoff on interface digests". The cutoff we built is global: one
digest for all rows. This record makes it per document.

## Decision

1. **A document's rows of round r are its rows of round r − 1 when nothing it read changed.**
   - Round r derives a document's rows from three things: its own local facts, which never change; the
     `__prev` rows it reads across a hop; and rows of the same round in its own document.
   - If the `__prev` rows it reads are the same in rounds r − 1 and r − 2, its round-r rows equal its round-(r − 1)
     rows. It is *clean*.
   - Every other document is *dirty*, and only dirty documents are derived again.

2. **What a document reads is recorded as dependency rows, by key.**
   - For each across rule (a hop atom `rel__prev(...)` with `@doc(b) != @doc(head)`), `cross_rounds` adds a
     sibling rule. Its head is `rr_dep__<rel>__<cols>(@doc(head), key...)`.
   - Its body is the rule's body without the hop atom and without the literals that need the hop atom's own
     variables.
   - The key is the hop atom's columns that the rest of the body binds, for example `(b, a)` for
     `found__prev(b, a, x)` and `(q)` for `qv__prev(q, k, v)`.
   - The sibling rule over-approximates what the rule read: every key it could have probed, matched or not. A
     new row under a probed key therefore makes the reader dirty, even if no row matched before.
   - A rule whose head node is bound only by the hop atom has no reader to name. It makes its relation
     *global*: any change to it makes every document dirty.

3. **Changed keys come from the rows the round already holds.**
   - The loop already sorts each hop relation's rows to digest them. The changed rows of a relation are the
     difference between rounds r − 1 and r − 2, in both directions, since a row can also disappear: a later round
     can falsify a negation.
   - Projecting the changed rows on each recorded key gives the changed keys.
   - The dirty documents of round r are those with a dependency row on a changed key, plus every document if
     a global relation changed.

4. **Clean documents' rows are carried, not derived (step 2).**
   - A clean document's rows of the cross-file part's relations are bound as the component's *closed* rows
     (`setfix`'s `rel ... closed`, 0058): of an earlier wave, never a delta.
   - Every rule whose head has a node column gets the guard `dirty(@doc(head))`.
   - A relation with no node column, or one in `Bounded::unbounded`, is always derived again in full.
   - The carried rows are the ones the loop already holds for `__prev`, so nothing is read back from a table.
     0058 measured that loading and indexing rows per round costs more than deriving them.

5. **Then k is raised (step 3),** to the depth the library needs (8 for the 84-file corpus), and the pending round
   (0047) is kept.

## Order of work

1. Dependency rows and the dirty set, computed and reported per round (`round r hop rows`: dirty d of n), with no
   cutoff. Measured on the 84-file corpus at k = 8: the dirty documents per round. Go on only if the dirty sets of
   the later rounds are a small part of the documents.
2. The cutoff: carried rows, the head guard, and the same results as without it. The test is a corpus whose call
   targets and types are equal with and without the cutoff.
3. Raise k, measured on Jedi agreement and CPU.

## Measured (step 1, 2026-11-07)

A one-file workspace that uses `contextlib`, `iter`, `next`, `sorted` and `min`, with 64 library documents.
`calls.rr` at k = 8, debug build, `RERAIL_CROSS_CUTOFF=measure`. The table shows the share of hop rows held by
what the next round would derive again:

```text
// round   documents changed   dirty documents (their hop rows)   dirty nodes (their hop rows)
0          64                  107  (97%)                         24,825  (51%)
1          107                 136  (98%)                          1,139  (11%)
2          57                   66  (96%)                          2,735  (19%)
3          66                   62  (96%)                          1,320   (5%)
4          57                   39  (93%)                            485   (3%)
5          27                   33  (85%)                            247   (8.5%)
6          31                   27  (78%)                            253   (1.6%)
7          23                   19  (49%)                             42   (0.3%)
8          2                     0                                     0
```

- **A document is too coarse.** builtins.pyi and typing.pyi hold most rows, and some row of theirs changes in
  almost every round. Cut by document, rounds 1 to 8 would derive 84% of the rows again. The gate of step 1 is
  not met.
- **A node may be fine enough.** Here the unit is the row's first node: a class, a call, a scope. After round 1,
  a round would derive 0.3% to 19% of the rows again. These are lower bounds: the dirty nodes are not yet closed
  over same-document reads (below), and step 2 measures them closed before the cut is built.
- **What it costs.** The dependency rules raise the k = 8 run from 42 s to 62 s CPU (82,050 dependency rows).
  They should run for dirty readers only. A clean reader's dependency rows are those of the round before.
- **Results are unchanged** with the dependency rules on (2 open calls, 22 typed nodes).
- The dirty sets were checked against the pending round: it changes rows that no dependency predicts. That is
  right, since the pending round adds `unconverged(1)`, which every reader of it reads.

**The rounds are not monotone.** They negate 16 relations that change with them (`pv`, `found`, `ifn`,
`call_target`, `aknown`, …), and rounds 1 to 6 each take back 10 to 855 rows. So a round cannot simply continue
the last one's fixpoint with the new `__prev` rows as deltas. Rows must be derived again, and the cut decides which.

**Closed over same-document reads of the component's own rows**, the dirty nodes' share of the hop rows is:

```text
// round      0     1     2     3     4     5     6     7
// closed   62%   26%   29%   13%    9%   10%    8%  1.4%
```

That is 12% of the rows of rounds 1 to 7, which leaves a cut room to save most of a later round. Two gaps remain:

- Reads of a lower component's rows in the same round are not closed over. A dependency rule on every such atom
  cost more than the run: past the 2-minute cap, against 67 s with same-component reads only. Step 2 needs them
  closed. It needs them as cheap reads, not as one rule per atom: a lower component's changed rows can be
  diffed the way hop rows are.
- A same-document read whose reader cannot be named is not counted.

The dependency rules cost 42 s → 67 s CPU at k = 8 over all readers. Step 2 runs them for dirty readers only.

A cut finer than a document needs something a document cut gets for free. Within a round, rows of one document read
each other: `found(c, a, x) :- base(c, b), found(b, a, x)`, with b in c's document, is not a hop. So a dirty node
makes dirty every node of its document that reads it in the same round. Step 2 therefore records same-document
reads as dependency rows too, and closes the dirty set over them before the round runs. Decision 6 below replaces
decision 4's unit.

6. **The unit is the node (revised after step 1).**
   - Dependency rows name the reader's node, not its document. They are recorded for same-document recursive
     atoms as for hops.
   - Before round r, the dirty nodes are closed over the same-document dependency rows of round r − 1.
   - A clean node reads only clean nodes and unchanged keys, so its rows are carried.
   - A dirty node's rules run with the guard `cut_dirty(head)`, an input relation joined first. Its
     recursive atoms read the carried rows through a variant atom `<rel>__carry`, so its rules fire in the first
     wave.

## Prior work consulted

- **03 §13:** "early cutoff on interface digests". This record takes the digest down to the document.
- **ast-index-spike, design index §20** (incremental maintenance): splice what changed rather than rewrite. The
  spike never built update or delete (R L1716-1812). The spike is per-file indexing, with no cross-file
  fixpoint, so it offers no prior form for this.
- **History index, ledger row 7:** sharing pays only for measured coincidence. Hence step 1 measures how often a
  document's inputs are unchanged before the cutoff is built.
- **0058, Measured:** loading rows into the engine each round costs more than deriving a few, so carried rows
  stay in memory.

## Not decided here

- Keeping the engine's store across rounds and deleting dirty documents' rows in place. Rebinding the clean rows
  as closed rows is the first form.
- Dependency rows for in-round reads of another document. Within a round, a node-keyed atom that may lie in another
  document is a hop (0035), so none exist beyond the unbounded relations, which are always derived in full.
