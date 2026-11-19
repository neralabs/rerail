# 0463. A level's formulas run on one kernel, in phases, from the batch's parents

Status: accepted (2026-11-19). Follows 0461. Supersedes 0459 (the size classes are removed with `label_formula`) and
0461's decision 1 (the phases are no longer a measure behind a switch, they are the only path).

## Intro

0461 measured a level's formulas as phases: one dispatch per phase, a thread per (word, formula). Its word steps cost
about 32 µs a dispatch for a whole batch, and its time was the one-hop pushes' chains: in format 16 a parent read is
a search back over up to 7 siblings. Its next steps were:

1. spread a phase's pushes over its SIMD group, as `label_micro` spreads its roots;
2. turn the descendant, following and reach steps into scans, so that a level runs on one kernel;
3. compare the phases with 0459's size classes, and remove the loser.

The user: "please look at this: [those three]. Also look at extracting the parent/child relationships for a block to
shared memory during the same kernel launch, so that the hopping and dependent reads are faster, or at least caching
them".

## Decision

1. **A batch's parents are computed once, by `label_parents`, before its first level.**
   - A threadgroup is a block of 64 words (2,048 nodes), a thread a word.
   - Into threadgroup memory goes each node's parent or a link to its previous sibling. Each is one slot read: the
     slot before a node names its parent (an inner node, or every 8th sibling) or its previous sibling.
   - A link out of the block is a *head*. The heads go on a list in threadgroup memory, and the group's threads take
     them one each, the tape's search from that sibling.
   - Three rounds of pointer jumping resolve every link in the block (a run of siblings between two that name the
     parent is at most 7).
   - The result is written once per batch into an array of the batch's nodes: 4 bytes a node of the region,
     `TAPE_NONE` for none. Under 0353 this is a derived fact (an axis's answer) written once where it is computed. It
     is not a copy of the tape.
2. **Every device formula runs on `label_phase`.**
   - A threadgroup is a block of 64 words and one formula; a dispatch is a phase.
   - The instructions (`phase_program`):

     | Steps | Instruction | How |
     |---|---|---|
     | sets, AND, OR | `ROW`, `AND`, `OR`, … | the thread's own word |
     | `CHILD`, `CHILD_IN` | `UP` | push to the parent |
     | `PARENT` | `DOWN` | pull: a node reads its parent's bit |
     | `FOLLOWING`, `FOLLOWING_IN`, `REACH_FOLLOWING` | `LAST`, then `BEFORE` | push each member to its parent's entry in a node array (the last or first member child, by atomic max); the next phase pulls |
     | `REACH_DESCENDANT` | `SPAN`, then `COVER` | two-phase running-max scan: each word's and block's furthest member end, then the blocks' prefix and the block's own |
     | `REACH_CHAIN` | levels of `SPAN`/`COVER` | as the CPU's `Reader`: two descendant scans a level |
     | `DESCENDANT`, `DESCENDANT_IN`, `CHAIN` | `CLIMB` | push up the parent array, stopping at an ancestor another climb set |

   - A node's parent is one load from `label_parents`' array.
   - Every word a device formula has compiles. `label_formula` is removed, and with it:
     - 0459's size classes and `Shape::wide`;
     - Q271's fused mode;
     - its profile counts and trace;
     - the switches `RERAIL_FORMULA_PHASES`, `_WIDE`, `_WIDTH`, `_FUSED`, `_TRACE` and `_ONE`.
3. **A chain's levels are the run's.**
   - A program holds one level at first (`PHASE_CHAIN_LEVELS`), which held every chain of s1000.
   - Its last level checks that no member is left. If one is, the run fails with `Error::ChainLevels` and runs again
     with twice the levels.
   - Past 64 levels the run goes to the interpreter, as a walk past its frames does.
4. **The preamble once a group.**
   - Two threads find the block's first and last files; each thread then searches only between them.
   - A program carries each phase's first instruction.
5. **The plan's launches:** a level's formulas count their leaves and the phases of the formula with the most. The
   batch's parents count once.

## Measured

Measured on the M5 Max, pinned to the GPU, with the memory line lifted and encoder timestamps
(`RERAIL_GPU_ENCODER_TIMES`). The rows were the same as native at every step: internal_dead_code (s1000 988d8fbe, big1
26aecb10) and public_api_triples (3ffb9c9f). The ops in these programs' formulas are 7, 9, 10, 14, 23 and 25.

**The parents in threadgroup memory, in the same launch** (the user's first form). Every formula was on `label_phase`,
each phase computing its block's parents itself:
- s1000: 25–36 ms, where `label_formula` took 19–26 ms;
- with the heads spread over the group: 20–33 ms;
- an ablation: the parents were 22 of 33 ms.

A block's parents were computed once per formula per phase, about 200 dispatches times the formulas, and the heads'
searches are the tape's.

**The parents once a batch** (the cached form). Same binary, alternating, load 32–45:

| | `label_formula` (before) | `label_phase` + `label_parents` |
|---|---|---|
| s1000 internal_dead_code | 23.7–28.9 ms | 12.9–13.0 + 2.2 ms |
| s1000 public_api_triples | 21.9–23.6 ms | 10.2–11.5 + 2.2 ms |
| big1 internal_dead_code | 5.3 ms | 1.9 + 0.34 ms |
| big1 public_api_triples | 5.1 ms | 1.5 + 0.34 ms |
| s1000 command buffers' GPU time | 68–72 ms | 56–62 ms |

`label_parents` costs about 185 µs a batch (12 batches on s1000).

**Where `label_phase`'s time is now** (s1000 internal_dead_code, about 200 dispatches, about 13 ms):

| Part | Cost |
|---|---|
| An empty dispatch | 5–7 µs (1.1–1.5 ms in all) |
| The preamble (each thread's file, whether its formula is wanted, the phase's start) | 2.5–4 ms |
| The epilogue (the targets' rows; mode 2 reads its rule's codes at set bits) | 2–3 ms |
| `DOWN` pulls | about 0.8 ms |
| `UP` pushes | under 0.2 ms |
| The sibling pushes | within the noise |

The preamble's change (decision 4) took big1 from 1.92 to 1.73 ms and from 1.56 to 1.40 ms. On s1000 it was within
the noise.

**Chain levels:** one held every chain of s1000. Two took 26 → 32 ms (measured before the parents were cached), with
18–49 more dispatches.

**The pushes spread over the SIMD group (item 1) is not built.** Once a parent is one load, a member's push is a few
loads, and the pushes together are under a millisecond. The tail item 1 was to cut was the parent search, and the
cache removed it.

## What is next

- `label_leaves` is now the largest of the formulas' GPU cost (7.4–11 ms on s1000).
- The epilogue's 2–3 ms.
- A `SPAN` and its `COVER` are a phase apart because a block's cover needs the blocks before it. A scan with decoupled
  look-back would put them in one dispatch, at the price of a wait between groups.
