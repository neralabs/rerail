# 0365. A walk starts within its anchor

Status: decided (2026-11-17); built on the CPU (the wave's root walks, and its label passes' walks in the levels
past the recursive groups) and on Metal (§8: the same walks, and the root records, from anchors the device
evaluates, their text tests reading the walks' text rows; and the formula passes of 0036 as device formulas, §9).
After 0334 §5 ("another anchor or driver"), which it
takes out of the replanning. The user, 2026-11-17, on that item: "please take a look at the ast-index-spike design
and journal, where this idea was comprehensively explored, and tell me what we should do with it now"; then, on the
recommendation below: "yes go ahead, making sure that you record the decision and the evidence it's based on".

## Intro

A matcher clause is lowered to a root program and label passes (`compile/matcher.rs`). Each runs as a walk from a
set of starts: its domain (`Domain`: every node, a root's kinds, or a label), past its entry (the kind tests the
program makes before it moves, `Entry`). A domain is every node wherever the pattern's root has no kind: a rule of
`any`, `not`, a regex or a relation alone. The walk then starts at every node of the file, and nearly all of them
emit nothing.

The ast-index-spike measured this as anchor rows per answer, and drove it to 1.05. The wave had never applied its
answer: on corpus0 the root walks started at 1,698 candidates per root.

## Prior work

The spike's design and history indexes (`docs/rule-engine-direction/prior-work/`), topic 5 (anchor selection) and
history 2.7, with the source lines read:

- **Anchor on the rarest mandatory concrete variable, not the root.** Root anchoring when the root is common took
  1 MB of Python from 3.9 s to 62.2 s (DH L1236-1246); leaf (`identifier`) anchoring had a 10 s tail (DH
  L1365-1383); the rarest variable gave 112 s → 33 s (D S049-S050 L3024-3112). [STABLE].
- **Anchor selection is finished at 1.05 anchor rows per answer** (S151, DH L2578-2597): "the kind anchor reads
  5,661,799 rows and the queries answer 5,407,388 of them ... What is left expensive is per-row cost in the tape
  walk, not row selection."
- **A value anchor before kind, where it is a read** (vidx, DH L1912-1970); a seed (value, word, complement) is
  scored against the whole pattern's rarest bound, not its own variable's (S205, DH L5074-5143: an exclusion seed
  that displaced an 83x rarer structural anchor; S209, DH L5466-5509).
- **Climb against descent is priced by work**, not kind counts or a fitted ratio (S277, DH L8691-8730: the leaf
  2.57x more common than the root was cheaper, since the root-down walk recursed; `efreq` per direct edge).
- **Plan choice by run and retry was built and removed** (D S069 L4110-4129): it spends the budget on the queries
  with least headroom, makes latency nondeterministic, and cannot batch.
- **Cardinality alone is insufficient where one direction implies a recursive walk; score every seed in one place**
  (history 2.7 (d)).

Decision 0026 already took the rule ("climb versus descent is a planning choice: anchor on the rarest kind, climb
toward the root, then descend into required children"). 0036 built its mechanism for label passes that are node-set
formulas (`compile_sets`): a formula's axis steps follow the inner set's members, the spike's leaf-up climb. What was
missing is the anchor for everything that is not a formula: the root walks (they capture, so they are never one) and
the label passes outside the fragment.

## Decision

1. **Every walk from a set of starts starts within its anchor.** A walk's anchor is `Set::anchor` of its program
   (`compile/sets.rs`): the conjunction of the tests it makes, read as 0036's formulas read them, with what the
   formula fragment does not hold dropped: a capture is skipped, and a next-sibling step, a backward jump, a
   repetition or any other operation ends the anchor's reading of its region. A region saved and restored drops
   only its own rest. A fork whose branch moves ends at the union of the branches.
2. **The anchor is a superset of the starts that emit, by construction.** Dropping a conjunct only widens a set,
   and every axis step's set grows with what it reads where it lands. A start outside the anchor emits nothing, so
   not walking it changes no root, no target and no count (a count's tally is over every node, and an unwalked
   start counts 0, as its walk would).
3. **The anchor is evaluated as a formula** (the `Reader`, with the tape's column sets shared with the formula
   passes): its rarest parts bound the work, its axis steps climb from their inner members. This is the spike's
   rarest anchor with climb and descent, in 0036's mechanism, chosen per walk from the program, with no
   statistics, threshold or ratio to fit: the formula is exact where the program is a formula and a superset
   otherwise.
4. **A text test is a value anchor only where it is a read.** The anchor reads the walk's own text rows where the
   walk computes them anyway (a label pass's micro walk), else the term index's row where the tape has it
   (`prefilter::terms`). It never scans text of its own: elsewhere the test is dropped from the anchor and the walk
   tests it at the starts it keeps. (Measured: with an anchor scan of its own, text scanning rose from about 1.8 to
   2.9 s CPU on the 10 files below, more than the anchors saved there.)
5. **Where anchors apply:** the wave's root walks (every label is final there) and its label passes' walks (micro
   walks, packed walks and interpreted passes) in the levels past the recursive groups, which read finished labels
   only. Not in a recursive group's rounds or sweeps: an anchor there would read the group's own labels as they
   settle, which through a complement are not monotone.
6. **It is not a replan.** 0334's replanning stays as built for walks that overrun; a misprediction still reprices
   its program. An anchor is the plan the walk starts with, and it is decided before any walk runs.
7. **`RERAIL_ANCHORS=off`** turns them off (read at each batch): the check against walks without them.
8. **On Metal the device evaluates the anchors** (`label_leaves`, `label_formula` in `metal_label_passes.metal`), since
   the host does no evaluation there. An anchor is lowered to postfix words over a stack of node sets (`Set::device`,
   `DeviceFormula`). A threadgroup takes a (file, formula).
   - **Leaves are shared.** The tests of a node alone (kind, field, flags, a grammar supertype) are read once per run
     for every formula: one pass decodes each node's code once and sets each distinct leaf's row. Evaluating every
     formula's tests over every node instead cost about what the walks it saved did (the walks also test the kind
     first and stop), so per-formula tests are not an option.
   - **A step inside a conjunction is priced by work** (S277), on the device, by the two sets' members: it reads down
     from the conjunction's members (their children, their subtree by a rank over the inner set, their next
     siblings), or it climbs from the inner set's members, whichever has fewer (down where the conjunction's members
     are at most 4× the inner set's). A parent is a search on the tape (the generated read loops back), and a child,
     a next sibling or a subtree's size is not, so climbing from every member was most of the cost: 3.35 M parent
     reads on 6 files before, 0.38 M after.
   - **A step over every node** (the anchor drops a branch's rest to every node, so `Child(All)` occurs) is a leaf by
     the node's own links: "has a node below" (in preorder, the same as "has a child"), "has a named child", "has a
     next sibling" or "has a next named sibling". No climb.
   - **A text test reads the walk's own row** (`TEXT`, by slot): the rows the level's walks (or the root records)
     read for that slot, keyed as they are (`Micro::slot_key`), bound to `label_formula`. So the device's anchor is
     the CPU's (§4). A fused pack's union keeps its members' text tests, each member's slots past those before it,
     each by its key: the row the pack's walk (or another walk) reads for the key. Where no walk reads one the test
     is every node, a superset.
   - Identical formulas of a level (by their lowered words: leaf, label and text rows) are evaluated once and written
     to each target. An anchor that tests only kinds and grammar supertypes is not built, since the rule over codes
     and the walk's entry test already cover them.
   - Where the anchors go:
     - **A label pass past the recursive groups.** Its rule over codes gets a row of bits (a new rule kind, 2) that
       `label_formula` sets at the start of the pass's level, after the levels before it have settled. `root_mask`
       ANDs the row into the rule, so every reader of the roots (micro walks, the interpreter, fused groups) is
       unchanged.
     - **The wave's listed roots** (`roots_batch`). `label_formula` clears the list's bits outside the anchor.
     - **The root records** (the engine's demand, `RootJobs`). Each walk's anchor is a row past its text rows, and a
       start outside it returns before it walks. A fused pack's anchor is the union of its members'.

9. **On Metal 0036's formula passes are device formulas** (`Pass::Formula`), not walks. A roots label pass past the
   recursive groups whose micro walk is a node-set formula (`Set::of`, exact) with no text test is evaluated by
   `label_formula` into the pass's scratch row, within its rule over codes (`root_mask`, mode 2), and settles as a
   program's does. `RERAIL_SETS=walk` walks them, as on the CPU (now read at each level, so one process can check
   both ways).

10. **On Metal a targets pass is its image** (`Pass::Formula`, `image`), past the recursive groups, where its
    program is one (`Set::targets`: the capture its last act; no text test; its roots a rule over codes). The
    formula starts from its roots' rule as tests (the domain's kinds and the program's entry tests) and follows the
    walk's steps, lowered as the device reads them:
    - a child step is `PARENT` (the nodes whose parent is a member), a parent step `CHILD` unnamed (the nodes with
      a member child);
    - descendants (`REACH_DESCENDANT`): in preorder a node lies below a member where a member before it ends past
      it. A block of words a thread takes its members' furthest end, the blocks' running maximum follows, then each
      word is masks, its members in order. Work follows the members, a word operation each;
    - later siblings (`REACH_FOLLOWING`): each member's next siblings, until one another member's run has set;
    - a chain (`REACH_CHAIN`): from each member down in preorder by subtree sizes, into the nodes of `through`
      only. A node another member's walk set is that walk's to go on from;
    - a named step is the unnamed image within the named nodes (`FLAGS` named), a chain's `through` too.

    It is written as it is (mode 3) into the pass's scratch row, which settles as the walk's captures did.
    `RERAIL_SETS=walk` walks it.

## Evidence

All on the M3 Max, debug build, the Python clauses of the pack's policies (491) in waves of 16, on corpus0
(`~/orca/rerail-shared/bench/step-vs-main/corpus0`).

**The census** (`anchor_rows_per_answer`, the spike's S151 measure; every clause active on every file, the
prefilter's file gate set aside):

| | starts | roots | starts per root |
|---|---|---|---|
| root walks, 50 files, before | 7,753,599 | 4,566 | 1,698 |
| root walks, 10 files, before | 1,122,666 | 980 | 1,146 |
| root walks, 10 files, within their anchors | 1,000 | 980 | **1.02** |

The worst clauses started at every node (SIM118 and RUF005: 135,743 candidates on 50 files, one root each). Every
root found was inside its anchor (the census asserts it on every tape). Label passes on 10 files, by route:

| passes | starts | within anchors | answers |
|---|---|---|---|
| formulas (0036), roots | 23,959,296 | (no walk) | 4,203,722 |
| micro walks, roots | 1,478,999 | 783,044 | 79,158 |
| micro walks, targets | 1,116,264 | 323,762 | 314,142 |
| micro walks, counts | 1,199,400 | 427,840 | 295,811 |

So most label passes already followed their members (0036), and the root walks were the gap. A count pass's tally
is over every node, so an unwalked start must count nothing: the census walks every count start outside its anchor
too and asserts none emits (it held on every tape).

**The check** (`anchored_walks_find_the_unanchored_roots`, 10 files, every wave run without and with anchors on
the CPU): the settled findings (roots, or W's matcher at the candidates where the clause went to W) are the same
for every clause and tape. The anchors kept 977,001 starts and dropped 2,182,620 (69%). Root walks took 4.46 s
thread CPU without and 1.48 s with (anchor evaluation included); the anchors' evaluation was 0.52 s CPU in all.
Three (clause, tape) went to W without anchors and two with: walks that overran from every node finish from their
anchors. The whole wave's process CPU was 97.7 s without and 86.5 s with; most of it (about 80 s of the labelling's
93 s) is outside the labelling's stages and the same both ways (work queue).

**On Metal** (M3 Max, debug build, corpus0; device time from `RERAIL_GPU_KERNEL_TIMES`):

- `on_metal_anchored_walks_find_the_unanchored_roots` (every third wave of a dozen clauses, 14 waves, on 6 files;
  each wave's roots and its root records from every node, anchors off and on, both on Metal): the same settled
  roots and the same records. Device time:

  | | label passes' walks | anchors (formulas, leaves) | total |
  |---|---|---|---|
  | without anchors | 2.16 s | | 2.16 s |
  | with anchors | 1.06 s | 0.54 s (0.34 s, 0.20 s) | 1.60 s |

  The steps that led there, each measured the same way (anchors' device time): a threadgroup per file over every
  formula in turn, 7.1 s; a threadgroup per (file, formula), 1.9 s; every step read downward from every node, 3.3 s;
  shared leaves, 1.7 s; steps priced by work, 0.84 s; steps over every node as leaves and the run's leaves read once,
  0.54 s.
- With text tests reading the walks' rows and 0036's formula passes as device formulas (§8, §9), the same check
  (its "off" now also walks the formula passes, `RERAIL_SETS=walk`): the same settled roots and records. 1,215
  formula passes ran as formulas, 736 text tests read a walk's row and none fell back to every node. Device time:

  | | label passes' walks | formulas (anchors and passes, leaves) | total |
  |---|---|---|---|
  | without (anchors off, formulas walked) | 2.15 s | | 2.15 s |
  | with | 0.98 s | 0.55 s (0.36 s, 0.19 s) | 1.53 s |

  The formula passes had already walked only from their anchors (a formula's anchor is about the formula), so
  running them as formulas saves little more device time (walks 1.06 → 0.98 s); what it removes is their walks.
  The recursive groups (`label_group_*`, 2.6 s here) are untouched: anchors apply past them.
- `spilled_walks_find_the_cpus_roots` (all 491 clauses in 41 waves, 12 files, every walk spilling on Metal, anchors
  on both sides): Metal's findings are the CPU's (1,023 roots), before §9 and again after.
- `lowered_clauses_find_the_same_roots_on_metal` (each of the 491 clauses alone, 3 files): 491 of 491 equal both
  ways; Metal took 76.0 s without anchors and 6.5 s with, the CPU 6.9 s and 2.6 s. After §9 (and text): 491 of
  491, Metal 5.7 s, the CPU 2.3 s.
- On the M1 Max the first check passes too (the same 21 roots and records).
- With targets passes as their images (§10) and a fused pack's text kept (§8), on the M3 Max:
  `on_metal_anchored_walks_find_the_unanchored_roots` gives the same 21 roots and records both ways (its "off"
  walks the targets passes too), the images' steps reaching from 85,674 members (later siblings) and 3,190
  (descendants); `lowered_clauses_find_the_same_roots_on_metal` 491 of 491 equal, Metal 4.2 s (4.4 before), 6,923
  launches (7,059: the targets passes' walks are gone); `spilled_walks_find_the_cpus_roots` (41 waves, 1,665 roots)
  passes; `the_label_kernels_compile_for_every_format` compiles the kernels with the new steps.

## Not yet

Done since: text tests on Metal (§8), 0036's formula passes as device formulas (§9), the count passes' anchors
measured (the census), targets passes as their images on Metal (§10), and a fused pack's anchor keeping its
members' text tests (§8).

1. **The recursive groups' walks** start from every root of their rule (§5): an anchor there would read the group's
   labels as they settle. A design is with the user (the work queue, Q133).
