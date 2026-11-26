# 0474. A level's formulas as phases, a thread a word (a measure)

Status: built (audited 2026-11-23). Accepted as a measure (2026-11-19); its decision 1 superseded by 0475, where the
phases are the only path Numbered 0461 on final-rerail-shape (renumbered in the roll-up: the hub has its own 0461).
(the switch `RERAIL_FORMULA_PHASES` is gone). Follows 0473. Its epilogue's rule test at set bits only (§3) is on for
`label_formula` too.

## Intro

`label_formula` runs each formula as a group per (file, formula). The group runs the formula's steps one after
another, with device barriers between them. After 0473 a dispatch's time is still the largest file's group running
that chain of steps. The user asked: "there should be a better way of doing this. Take a step back and tell me what
the optimal launch shape is and how we could do a fair amount of work per thread whilst using the full parllelism and
avoiding the overhead of too many launches".

The answer was to drop the file as the unit of work:

- A level becomes a few phases. Each phase is one dispatch for all of the level's formulas, over the batch's whole
  range of words.
- A thread is a (word, formula) pair.
- A step is word-local (sets, AND, OR), one hop (parents or children of members), or a range (descendants,
  following, reaches: a scan).

The first measure was to be one phase shape for the word-local and one-hop steps, on s1000 and on the large file
alone, with the rows checked against native. The user: "yes do that".

## What was built

- **`phase_program`** compiles a formula to `label_phase`'s instructions: `ROW`, `AND`, `OR`, `ZERO`, `UP`, `DOWN`
  and `END`. It returns `None` for any other op, and such a formula stays on `label_formula`.
  - A one-hop step pushes from the members in the thread's own word into a fresh slot, cleared before phase 0.
  - Its set is whole only in the next phase. A value read after its push ends that phase.
  - `CHILD_IN` takes its candidates when its value is read.
  - A freed slot is used again only after the next phase boundary.
- **`label_phase`** takes one dispatch for each phase of a level's formulas, over the batch's whole range of words.
  - A thread finds its word's file by a search and checks whether its formula is wanted in that file.
  - It runs the phase's instructions on its word.
  - After the last phase it runs `label_formula`'s epilogue.
  - The slots are in one scratch buffer for the run, at most `FORMULA_SCRATCH_BYTES`.
- **The pushes gather in a register.** Bits for one word of the target go out in one atomic when the next bit
  falls in another word. Siblings share a parent, and a member's children run through a few words.

The rows were the same as native for internal_dead_code (s1000 988d8fbe, big1 26aecb10) and public_api_triples
(3ffb9c9f), with the phases on and off.

## Measured

Measured on the M5 Max, pinned to the GPU, with the memory line lifted and encoder timestamps
(`RERAIL_GPU_ENCODER_TIMES`). big1 is the largest file alone (`schema_transaction.py`: 88,927 nodes, 2,779 words).

| internal_dead_code | old path | phases on: `label_phase` | phases on: the rest on `label_formula` |
|---|---|---|---|
| big1 | 6.4–6.6 ms | 2.6 ms (3.1 before the gather) | 3.1 ms |
| s1000 (load 50–80) | 31–37 ms | 15–18 ms | 16–17 ms |

On big1 the phased formulas take 2.6 ms where they took about 3.3 ms. On s1000 they gain nothing.

**Where the phases' time goes.** These are ablations: rows are wrong while a part is skipped.

- On big1, 8 dispatches, 3.12 ms in all:
  - the epilogue's rule test took 1.07 ms, and 0.47 ms once it tests set bits only;
  - the UP pushes took 0.85 ms, and 0.62 ms with the gather;
  - the DOWN pushes took 0.69 ms;
  - with all three skipped, 0.67 ms is left.
- On s1000, 156 dispatches:
  - the floor is 5.0 ms, 32 µs a dispatch;
  - the pushes take about 10 ms;
  - the epilogue takes 2–5 ms.

**Tried and dropped:**

- **32 threads a word**, each pushing its own node: the push share halves, but the floor rises 6×, to 4.3 ms on
  big1 and 27 ms on s1000. A SIMD group's fixed work is the floor, so 32 times the groups costs 32 times it.
- **`PARENT` as a pull**, each node reading its parent's bit: 3.1 → 5.7 ms on big1. In format 16 a parent read is
  a search back over up to `K - 1` = 7 siblings' slots, a chain of dependent reads, so 32 parent reads a thread cost
  more than walking a few members' children.

## Decision

1. `label_phase` and `phase_program` stay, behind `RERAIL_FORMULA_PHASES=1`, off by default: on s1000 they do not
   yet beat `label_formula`. A unit test (`a_formula_is_compiled_to_phases_of_word_and_push_steps`) checks the
   compiler's phases.
2. The launch shape holds. The word steps cost about 32 µs a dispatch for a whole batch and all of its formulas.
   What is left is the one-hop pushes. Each is a chain within one thread: up to 7 dependent hops a parent, and one
   hop a child.
3. **`label_formula`'s epilogue reads its rule only at the set bits** (`root_mask_among` in `device_tape.h`;
   `root_mask_in` is a call of it). On s1000 the default path's formula encoders were lower than 0473's engine in
   10 of 12 alternating pairs (load 34–71). The GPU's clock moves with the load, so the range is wide:
   internal_dead_code 29.0–37.2 → 6.4–30.9 ms, and public_api_triples 14.4–30.0 → 5.9–26.3 ms. The rows were the
   same.

## What is next

- **Spread a step's members over the SIMD group.** `simd_roots` already does this for `label_micro`'s roots. It
  cuts a skewed word's chain without more groups.
- **Range steps as scans between blocks.** These are the ops a formula still needs `label_formula` for. Until they
  exist, a level runs on both kernels.
- **A cheaper parent.** A parent is a search in format 16 by design (the slots' size against `K`). An `UP` step
  could pull from the parent's side, a walk over its children, where its members are dense.
