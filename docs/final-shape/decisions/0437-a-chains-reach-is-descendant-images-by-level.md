# 0437. A chain's reach is descendant images, a level each time a member sits below a blocker

Status: accepted (2026-11-18). Follows 0419; closes Q318.

## Intro

`label_formula` evaluates a formula's set operations for one (file, formula) pair in a threadgroup (0365 §10, 0419).
A chain's reach (`REACH_CHAIN`) is one of its steps: the nodes reached from `from` by going down through `through`.
The CPU's `Reader` reads it in preorder: a node is reached where its parent is in `from`, or its parent is in
`through` and reached.

0419 found this step to be `label_formula`'s long pole: about 1.1 ms a pass, against 23–68 µs for the other steps.
Each thread took a block of words and read every node's parent (`tape_parent`). That read is a search of the tape
(CLIF's `parent`, a cold read), so each node of the file paid a chain of dependent reads. The user, on Q318: "I
think this is mostly fixed by using an accessor".

## Decision

1. **A chain's reach is read through the hot `tape_size` accessor, as descendant images.** Call a node not in
   `through` and not in `from` a blocker. Then a node is reached where its nearest ancestor among the members and
   the blockers is a member. A node's subtree is a range in preorder, `n + 1 .. n + tape_size(n)`. So the reach is
   built from descendant images, the step a targets image already reads (op 23, 0365 §10):

   R = ⋃ₖ desc(Fₖ) ∖ desc(Xₖ), with F₀ = `from`, Xₖ = blockers ∩ desc(Fₖ) and Fₖ₊₁ = `from` ∩ desc(Xₖ).

   The terms are disjoint: desc(Fₖ₊₁) lies inside desc(Xₖ), which is taken out first. The loop stops when Fₖ is empty.
   That is one level for each time a member sits below a blocker below a member, typically one or two.
2. **Each level is two scans, `formula_descend`.** A scan is op 23's: each thread takes a block of words, takes its
   members' furthest end, then a running maximum over the blocks, then each word by masks. Each thread reads and
   writes only its own block's words, so the levels need threadgroup barriers only. The step adds no dispatch, and so
   no launch against `KERNEL_LAUNCH_LIMIT`.
3. **The step takes one more slot:** from, through (turned into the blockers in place, since it is dead after the
   step), the reach, and the current level. `Set::emit` reserves 4 slots for it, up from 3.
4. **The CPU keeps its preorder pass.** On one thread, a pass in preorder that reads the parent is linear. The device's
   form is chosen because it has independent reads and work that follows the members; the CPU has no group of lanes
   to spread them over.

## Tried and not kept: reading down by child reads

Before this, the step was rewritten top-down. The members' children are reached, then the children of each reached
node in `through`. A node went on once, by clearing its `through` bit, through the hot child reads `tape_first_child`
and `tape_next`, a round a level of the tree with barriers. Its rows were the same. It was as slow: 0.5–9 ms a pass
against 0.8–5 ms. Each reached node's children are a chain of dependent reads, the cost the parent search had.

## Measured

s1000, pinned to the GPU (`RERAIL_LANE_PIN=gpu`), with the memory line lifted (`RERAIL_RUN_MB=20000`), in
alternating runs. The first run after the Metal source change was discarded. A baseline engine was built from the
same commit with the old kernel.

- **Rows:** on Metal the same as native on the same build: internal_dead_code 988d8fbe, public_api_triples
  3ffb9c9f. Both hashes moved with the hub merge (eca2b17c5), not with this change.
- **The step** (`RERAIL_FORMULA_TRACE`, `RERAIL_FORMULA_ONE`: one formula a dispatch; internal_dead_code; load 51–64).
  The pass count follows the run's chunks:

  | | Chain passes | Each | In all |
  |---|---|---|---|
  | Before | 18 | 0.8–6.8 ms | 32.4–56.9 ms |
  | After | 18–24 | 22–225 µs | 1.7–2.3 ms |

  A chain's pass is now at the floor of a dispatch: a descendants image (op 23) takes 102 µs a pass in the same runs.
- **`label_formula` in a run** (`RERAIL_GPU_KERNEL_TIMES`, three alternating pairs, load 42–62):
  - internal_dead_code: 56.7–87.9 ms before, 45.0–52.9 ms after;
  - public_api_triples: 54.2–80.5 ms before, 32.1–34.6 ms after.
- **The run's GPU time** (command buffers' timestamps) did not move measurably:
  - internal_dead_code: 73.8–85.2 ms before, 81.0–90.0 ms after;
  - public_api_triples: 37.3–42.9 ms before, 34.0–42.5 ms after.

  CPU is the same: internal_dead_code 5.8–6.0 s before and 5.4–6.1 s after, public_api_triples 4.4–4.7 s and
  4.5–4.8 s. The chain's dispatches overlapped other work in the run, so the kernel's own time fell without shortening
  the run's span.

## What is left

- **The children of members** (op 9, `tape_first_child` then `tape_next` per member) is now the largest step in
  one-formula mode: 103 ms over 224 passes, 461 µs a pass. It is the same shape, a dependent chain of child reads on
  one thread per member (Q324).
