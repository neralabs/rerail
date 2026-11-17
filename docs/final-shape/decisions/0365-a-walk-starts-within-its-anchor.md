# 0365. A walk starts within its anchor

Status: decided (2026-11-17); built on the CPU (the wave's root walks, and its label passes' walks in the levels
past the recursive groups); Metal is the next step (Not yet). After 0334 §5 ("another anchor or driver"), which it
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
| micro walks, counts | 1,199,400 | (not measured) | 295,811 |

So most label passes already followed their members (0036), and the root walks were the gap.

**The check** (`anchored_walks_find_the_unanchored_roots`, 10 files, every wave run without and with anchors on
the CPU): the settled findings (roots, or W's matcher at the candidates where the clause went to W) are the same
for every clause and tape. The anchors kept 977,001 starts and dropped 2,182,620 (69%). Root walks took 4.46 s
thread CPU without and 1.48 s with (anchor evaluation included); the anchors' evaluation was 0.52 s CPU in all.
Three (clause, tape) went to W without anchors and two with: walks that overran from every node finish from their
anchors. The whole wave's process CPU was 97.7 s without and 86.5 s with; most of it (about 80 s of the labelling's
93 s) is outside the labelling's stages and the same both ways (work queue).

## Not yet

1. **Metal.** On Metal every label pass is a micro walk, one thread per (pass, start), and the root walks' starts
   are a rule over each document's kind codes read on the device (`Roots::Codes`). No formula runs on the device,
   and the host does no evaluation on Metal. So the anchors there are the device evaluating `Set` formulas
   (`Kind`, `Class`, `Text`, `Field`, `Flags`, and the axis steps) into a label the walks start from; that is
   also 0036's formulas on Metal, whose passes run as walks today.
2. The count passes' anchors are applied but were not measured in the census.
