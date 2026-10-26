# 0007: Relations read tree neighbourhoods from the tape; anchored inner plans keep their postings

Status: accepted
Date: 2026-10-26

## Question
An inner plan with no kind (under a field-less `has`, a recursive `has` or a recursive `inside`) was evaluated over every node. Relating every node then decoded every node's parent. Where should such a plan get its candidates, and when are postings better? This affects all three targets, through anchor selection.

## Prior work consulted
- ast-index-spike design index, topic 1: preorder plus subtree size. The parent delta and size are in the 6-byte record, so a descendant test is an interval and a parent is one record read (D S012 L313-332, D S047 L2983-3005).
- Topic 2: no node table, and no per-query node table (D S072 L4241-4285, D S037 L2442-2593). Ordinals and parents are computed per involved parent only, because global windows are 75x slower than decode (D S037). Climb versus descent is a cost question: the rarest anchor plus a size-bounded descent beat a leaf climb, 112 s to 33 s (D S050, D S057).
- Topic 8: `nera_kids`, a native child walk that reads the tape in place, became the default after the blob-stepping fix (D S058 L3539-3580, D S063 L3737-3770, D S064 L3771-3816).
- Topic 7: an arbitrary-depth descendant is an interval test with no recursion (D S012).

Agreement: agrees. `nera_tree` is `nera_kids` plus a parent climb.

## Decision
- **`nera_tree(fid, node, axis)` reads a node's children (sibling skips) or ancestors (parent deltas) from the tape.** No relation decodes every node's parent.
- **An unanchored inner plan is evaluated over the scope's neighbourhood.** A field-less `has` uses the children, a recursive `has` the subtree intervals, a recursive `inside` the ancestors.
- **An anchored inner plan keeps its postings, as the rarest-anchor rule says.** A plan is anchored when it selects from postings before reading its scope:
  - a root kind or pattern;
  - a conjunction with any anchored member, which it evaluates first;
  - a disjunction of anchored branches;
  - a non-recursive `inside`, or a field-less `has`, with an anchored inner plan.

  Over every node, such an `inside` reads its inner postings' children.

## Evidence
shape-fixes.md §15. Anchor lab: 12 captured batches, 1,984 files, rows identical for every plan. SQLite VM steps fell in 5 of 7 representative policies and were unchanged in the other two:
- legacy_execution_surfaces −57%;
- serverplan_quality −36%;
- internal_dead_code −22%;
- target_architecture −16%;
- tripwires −15%.

The first version narrowed every root-kind-less plan. That included an `any` of kinds, and climbing a common posting's ancestors cost +36% steps on internal_dead_code: the climb-versus-anchor trade from topic 2.

## Consequences
- A plan the anchored test misses still gets the neighbourhood, which is bounded by the scope and is never every node.
- Neither path uses the cost statistics (`efreq`, the spike's S277 reverse anchor). The choice is structural, so a very large scope with a rare unanchored inner plan could still prefer postings.
