# 0683: The corpus batch shares the chunks' projections

Date: 2026-11-24. Follows decision 0682 (a chunk makes its documents ahead), decision 0616 (a session's corpus batch
takes its unchanged groups from the last run) and decision 0458 (the cross part reads its base relations off the
corpus batch).

## Intro

A corpus run evaluates its files in chunks. Each chunk's worker projects the chunk's documents into a batch (0682).
After the chunks, the cross-file part reads its base relations (`line`, `col`, `node`, the exports' documents) off
one more batch, the corpus batch, which holds every document of the run (0458). The chunks handed it their documents'
inputs, not what they had made of them. So the corpus batch projected every document a second time: on PostHog,
204,820 projection spans and 6.2 s of the cross part's 12.1 s, in a phase with little parallelism.

Decision 0353 says every reader reads a document's projection where it was made. 0616 already lets one batch share
another's made documents (a session's next run shares its last run's). This decision makes the corpus batch share
the chunks' made documents the same way.

## Decision

1. **A chunk hands on its made documents.** At its end, a chunk whose run has a cross part takes its batch's made
   documents (`Batch::projections`: the shared slots, each with its projection, and the projection they were made
   with). The batch it takes them from is its grower's last, which holds every round's documents. The run collects
   them beside the chunk's documents (`Evaluated::projections`).
2. **The corpus batch shares them.** It is made by one constructor, `with_matchers_from`, given what it takes from
   earlier batches (`Earlier`): a session's held batch (0616) and the chunks' projections. A document it holds as the
   chunk held it (key, digest, path and tag), that it has not taken from a held batch, shares the chunk's slot. Its
   projection is not made again, and keeps the chunk's block, as a renewed batch's do (0616).
3. **Only a projection made alike is shared.** The cross part's projection reads other relations than a chunk's (it
   adds the measures' keys, 0364 R6, and drops `tree_digest` and `def_digest`). Most reads do not shape a document's
   projection, only its rows. Those that do are one struct, `Making`: whether `syntax_error`, a token view, `text`, a
   source or line span's text and `end_point` are read. `Docs::project` takes its answers from it. Two projections
   make every document alike when their `Making`s and everything but their reads are equal (`Making::alike`). A
   chunk's batch that is not alike shares nothing, and its documents are projected again.
4. **The rows are still the corpus batch's.** Its relations' rows are made from the shared projections in its own
   groups. Atoms are the store's (`FactStore::atoms_handle`), one table for the run, so a projection's interned texts
   read the same in either batch.

`with_matchers_renewed` and the test-only `Batch::renewed` are replaced by `with_matchers_from` and `Batch::after`,
which take `Earlier`.

## Measured

PostHog, `run-rr` with `len-_py`, CPU, on the M3, run in turns (0431 step 7, head, head, 0431 step 7). Instructions
and peak are stable from run to run. The M3 ran other work meanwhile: the loads at the runs' starts were 36, 11, 12
and 20, so wall and CPU are rough.

| Run | Instructions | Peak | Wall | User CPU | stats wall | cross wall |
|---|---|---|---|---|---|---|
| 0431 step 7 | 3,146 G (3,102 and 3,190) | 2.35 GB | 130–143 s | 240–253 s | 2.0 s | 2.5 s |
| 0682 (5bddb66b9d) | 3,424 G | 5.66 GB | 70–86 s | 265–278 s | 2.4 s | 12.1 s |
| this decision | 3,286 G (3,279 and 3,293) | 6.42 GB (6.36 and 6.49) | 52–66 s | 247–271 s | 2.1 s | 5.9 s |

- All 30,041 documents the chunks made are shared (156 of 156 chunks' batches made alike). The cross part projects
  the 1,169 documents no chunk made, for no measurable time, where it projected 204,820 spans' worth in 6.2 s.
- The cross part went from 12.1 to 5.9 s, and the run from 3,424 to 3,286 G instructions.
- The peak rose 0.76 GB. The chunks' projections, and the blocks they are in, now live from each chunk's end until
  the cross part has read them, beside the chunks still running. The peak is still in the cross part's retain, at
  6.0 GB sampled. At that moment the host blocks held 6.9 GB, 5.65 GB of it free on their lists.

Against 0431 step 7 the run retires 4.5% more instructions and peaks at 2.7 times its memory.

## Requirements

- R1: a corpus batch that shares its chunks' projections has a fresh batch's rows in every relation, and projects
  none of the shared documents (`a_corpus_batch_shares_the_chunks_projections_where_they_are_made_alike`).
- R2: a chunk's batch whose projection makes a document otherwise (`end_point` read) shares none (same test).
