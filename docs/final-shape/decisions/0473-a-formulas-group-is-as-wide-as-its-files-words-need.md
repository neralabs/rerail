# 0473. A formula's group is as wide as its file's words need

Status: superseded by 0475 (2026-11-19): `label_formula` is removed, and with it the size classes. Was accepted Numbered 0459 on final-rerail-shape (renumbered in the roll-up: the hub has its own 0459).
(2026-11-19). Follows 0449 and 0440 (merged). The first measure of the plan for the label DAG on
the work queue (0027).

## Intro

`label_formula` runs a level's formulas (0365): a threadgroup for each (file, formula), the formula's steps one after
another, each a pass over the file's words between device barriers. After 0449 it was the label passes' largest GPU
cost, and its time follows the GPU's clock. The user: "go ahead with that plan", which was to run the chunk's label
DAG (leaves, formulas, settles, with count dependencies) as one persistent dispatch on the work queue, after one
level was measured as queue items.

Before the queue was built, these measures were made to see where a dispatch's time goes:

- **It is one group's time, not the work.** A batch's formula dispatches read 822k children in all, in 45–60 ms:
  about 12 reads a µs. One dispatch of one 24-step formula over 113 files took 2.7 ms. That is 113 groups, far under
  what the GPU holds at once.
- **The group is the largest file's.** A corpus of that file alone (`schema_transaction.py`, 88,927 nodes, 2,779
  words) took 17.7–19.0 ms over its 9 dispatches, and the same formula 2.3–2.8 ms. That is about 100 µs a step, about
  22 words a thread at 128 threads, about 4 µs a word: four SIMD groups on one core, with nothing to hide a load's
  latency behind.
- **Not the atomics.** Plain loads and stores in the word-owned steps (sets, leaves, AND, OR) changed nothing: 49.5–55.7
  ms against 50.1–53.4 ms.
- **Width moves it, both ways.**
  - One width for every group: 32 threads took 88.8–96.4 ms, 64 took 56.2–80.7, 128 took 41.0–54.2, 256 took
    42.5–54.3, 512 took 68.0–78.5.
  - On the large file alone: 1024 threads took 6.5 ms where 128 took 17.7–19.0. Small files leave a wide group's
    threads idle, and a wide group takes a core's room.
- **A queue of (file, formula) items would not change this.** The largest file's group runs the same steps in the same
  order whichever worker takes it. A level's other items can already run beside it, on the other cores.

## Decision

1. **A batch's files are in two size classes** (`formula_classes`). A file of at least 4 words a thread of
   `FORMULA_WIDTH` (512 words, 16,384 nodes) is large. A batch of small files only is one dispatch at 128 threads; a
   batch of large files only is one at the most the pipeline holds (1024).
2. **A batch with both is two dispatches in one encoder, concurrent** (`MTLDispatchType::Concurrent`): the large
   files' groups at 1024 threads, then the rest at 128. The kernel's `Shape::wide` names the class. A group whose
   file is in the other class returns before its first barrier. The two dispatches touch nothing in common: a file's
   scratch is its own region, and the rows' words are written by atomics.
3. The plan counts a level's formulas as three launches (the leaves, and a dispatch for each class).
   `RERAIL_FORMULA_WIDE=off` gives one dispatch at 128 threads, as before.

## Measured

s1000, pinned to the GPU, the memory line lifted, encoder timestamps (`RERAIL_GPU_ENCODER_TIMES`, no split
submissions), alternating runs against the merged engine before (load 27–44). The first run after the change was
discarded. The rows were the same as before on Metal and native: internal_dead_code 988d8fbe, public_api_triples
3ffb9c9f.

| `label_formula`'s encoders | before | after |
|---|---|---|
| internal_dead_code | 30.1–33.0 ms | 24.6–28.6 ms |
| public_api_triples | 23.2–35.5 ms | 17.3–28.3 ms |

The two classes as serial encoders gained nothing (26.0–34.1 ms): the large files' groups had already run beside the
small ones, and the second dispatch only added its own time. Run with split submissions (`RERAIL_GPU_KERNEL_TIMES`),
two dispatches in a row each pay a submission, which hides the gain.

## What is left, and the queue

The floor is now the large file's group at 1024 threads: about 30 µs a step, 0.4–0.9 ms a formula. The time is the
length of its chain of steps, so it falls only when a large file's step runs over several cores at once. In the
queue's terms an item is then a block of a step's words, (file, formula, step, block), and a step waits on a count of
the blocks of the step before it. A step that reads across blocks (a descendant's ranks, a reach's running maximum)
needs a scan between blocks. That is the next measure, before the level DAG: it is where this file's time is, and a
DAG of whole (file, formula) items keeps the chain.

## On rr-align (roll-up 11, Q369)

The label passes launch through the device layer (`lane`, 0440 step C). The lane has no concurrent dispatch, so a
batch with both size classes is two launches of `label_formula` in a row, the large files' first. They read and
write apart, so the rows are the same. Whether running them concurrently matters is open (Q369).
