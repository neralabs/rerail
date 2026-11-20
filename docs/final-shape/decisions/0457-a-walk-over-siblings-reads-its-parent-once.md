# 0457. A walk over a node's children or siblings reads its parent once, not at each hop

Status: accepted (2026-11-19). Follows 0437; closes Q324. (0440 until 2026-11-19: CUDA-support's 0440 is UKL.)

## Intro

After 0437, the largest step in `label_formula` was the children of members (op 9): 461 µs a pass in one-formula
mode, against about 100 µs for a descendants image. The user: "please do this" (Q324).

The step walked each member's children with `tape_first_child`, then `tape_next`. CLIF's `next` reads the node's
parent (`tape_parent`, a search of the tape, the cold read 0437 removed from a chain's reach) to find where the
siblings end, then takes `tape_next_in` against that end. So every hop paid a parent search, although the walk
already knew the parent: it started at it.

## Decision

1. **A walk over children takes the parent's end once** (`tape_end_of`, a hot read) and hops by `tape_next_in`
   against it. This covers:
   - the children of members (op 9);
   - the earlier siblings, both alone (op 10) and in a conjunction;
   - a step within a conjunction (ops 14–16);
   - "has a named child" (op 21).
2. **A walk over siblings reads the parent once** (`tape_siblings_end`, in `metal_tape.metal`), then hops the same way.
   This covers the later siblings (op 24), "has a named later sibling" (op 22), and a conjunction's sibling step.
3. **No `tape_next` is left in a loop in `label_formula`.** "Has a next sibling" (op 18) still calls it once a node:
   one parent read, not a chain.

This is the same rule as 0437: a step reads the tape through the hot accessors. A cold read stays only where nothing
cheaper says the same thing, and then once, not inside a loop.

## Measured

s1000 internal_dead_code, pinned to the GPU, with the memory line lifted, alternating against an engine built from the
commit before (89017cb82). The first run after the Metal change was discarded. Rows on Metal are the same as native on
the same build: 988d8fbe and 3ffb9c9f.

- **One formula a dispatch** (`RERAIL_FORMULA_TRACE`, `RERAIL_FORMULA_ONE`; three runs each, load 32–46). Medians by
  formula shape, about 20 passes each:

  | Shape | Before | After |
  |---|---|---|
  | `13.9.4.5` | 810 µs | 207 µs |
  | `13.9.13.4.5` | 846 µs | 212 µs |
  | `13.13.6.20.13.9.13.5.13.6.5` | 636 µs | 133 µs |
  | `13.9` | 431 µs | 184 µs |
  | `13` (one leaf read, no step) | 333 µs | 244 µs |

  The op-9 shapes are now at the floor a single leaf read sets. The leaf-only shape came out lower too, so about a
  quarter of each gap is noise between runs. Formulas with op 9 took 287 ms in all before and 198 ms after; the whole
  trace took 503 ms before and 407 ms after.
- **A run, without per-kernel timing** (four pairs, load 14–16):
  - the matcher waves' GPU time: 64.9–71.7 ms before, 58.9–65.3 ms after, lower in every pair;
  - the run's GPU span: 61.8–85.1 ms before, 62.5–82.6 ms after, the same range;
  - CPU: 5.1–5.6 s before, 5.5–6.7 s after. The 6.7 s was one run, at a wall of 8.2 s.
- **With per-kernel timing** (`RERAIL_GPU_KERNEL_TIMES`, three pairs, load 24–32), `label_formula` took 40–44 ms
  before and 26–38 ms after on internal_dead_code, and 20–38 ms before against 19–38 ms after on public_api_triples.
  In this mode internal_dead_code's span came out higher, 64–74 ms before against 83–93 ms after. The label kernels'
  own sum was lower (76.7 against 63.8 ms), and the runs split their waves over the chunks differently. Without the
  timing the spans are the same, as above. So the timing mode's span is not a measure of this change.

## What is left

- The plain formulas' floor in one-formula mode, 240–330 µs, is now the most common time a step formula takes. In a
  run, the floor is spread over a level's formulas (0419).
