# 0682: A chunk makes its documents ahead, and counts its semi-joins; only the counts are kept

Numbered 0680 where it was written (5bddb66b9d); renumbered, Native-and-GPU-strata holding 0680 and Parser-generators 0681.

Date: 2026-11-24. Follows decision 0458 (the base relations' statistics), decision 0494 (the batch made by group)
and 2bc2d88258 (t50 item 1: the cross part's statistics from the chunks).

## Intro

A corpus run evaluates its files in chunks. Each chunk's documents are projected into a batch by a worker that runs
ahead of the chunk's evaluation. Decision 0494 made the batch lazy: a document's rows are made when a read first asks
for them. On PostHog that meant 30 s of making rows and 6.3 s of projecting documents inside the chunks' joins, and
almost nothing (0.17 s) on the worker that runs ahead.

After the chunks, the stats phase counts the base relations the cross-file part reads. A semi-join relation (`line`,
`col`, `node`, `ast_depth`) is read by the cross part only at the nodes the exports hold, so its statistics are those
rows'. The chunks counted them at their own exports' nodes and handed the rows on (`ChunkHeld`). But a chunk saw only
the exports it did not hand off as they were derived, so the stats phase walked the rest of PostHog's 8.6 M held nodes
again, which made 29,260 documents. It kept every held node and its row for the next run (`SemijoinRows`: 26 M words
for one relation).

## Decision

1. **A chunk's documents are made on the worker that runs ahead.** The job that projects a chunk makes every
   document's rows of its batch, as one group (`Batch::make_every`), before the chunk is handed to its evaluation.
   That is what `Batch::build` did before decision 0494, now into the batch's block relations.
2. **A chunk counts its semi-joins, and hands on counts, not rows.** `SemijoinCounts` is counted by document: each
   document's rows at the held nodes, its every row, and each column's count of each value in it. Counts of disjoint
   documents add, and a document's are dropped whole. A chunk counts the documents it claims at the nodes all its
   exports hold, the handed-off ones included, and counts each document's every row from its rows' offsets
   (`documents_of`, no row made).
3. **The stats phase adds the counts up.** A semi-join's counts are the chunks' counts, then the kept counts of the
   documents no chunk counted and no edit changed. Only documents that neither covers are walked, at their held
   nodes. The exports are visited for held nodes only when there is such a document. The relation's whole count is
   the counted every rows, not the corpus batch's `len`, which projected every document again.
4. **Only the counts are kept.** They are kept beside the documents' table, with the memo plan they were counted
   under: an unchanged file's exports hold the same nodes only under the same plan, as its kept inputs do. A run of the
   same documents and plan takes the kept statistics as they are. An edit takes the kept counts of the documents it did
   not change. `SemijoinRows`, `ChunkHeld`, the run-wide set of held nodes and the exports' digest that keyed them are
   gone.

   *Amended 2026-11-25:* the counts are kept as their maps hold them, with no sort (the host never sorts).
   `SemijoinCounts::encode` sorted every (value, document) count first, on the main thread at the end of the cross
   part: on PostHog about 1.1 s of the main thread, 840 of its 1,091 samples in the sort. The maps' hasher has no seed
   and the chunks' counts are added in the order the chunks are filed, so the words are the same on every run without
   it. On PostHog, in turns on the M3 (after the hub merge 41654b2e52), the cross part's main thread went from
   5.19–5.68 s to 3.80–4.38 s, and the run from 2,415–2,416 G to 2,407–2,409 G instructions. The peak stayed within
   its noise (7.35–7.44 GB, then 7.45–7.50 GB).
5. **A view's keyed walk probes its keys.** `for_rows_keyed` on a view finds each key's row by its node and reads the
   rows in row order, where it read every row of each document a key named. A token's view (`op`), whose rows are not
   found by their node, is still walked.

## Measured

PostHog, `run-rr` with `len-_py`, CPU, on the M3. Instructions and peak are stable from run to run. Wall and CPU vary
with the machine's load. The wall and CPU here are from the run at load 7–12. 0431 step 7 is the mean of its four
runs today.

| Run | Instructions | Peak | Wall | User CPU | stats wall | cross wall |
|---|---|---|---|---|---|---|
| 0431 step 7 | 3,153 G | 2.36 GB | 128–159 s | 240–245 s | 2.0 s | 2.5 s |
| hub merged (33d5a8e668) | 3,685 G | 7.40 GB | 60 s | 273 s | 9.0 s | 4.8 s |
| counts in the chunks (items 2 and 4), before the every-row count | 3,406 G | 5.67 GB | 73 s | 262 s | 8.5 s | 5.8 s |
| items 1–5 | 3,424 G | 5.66 GB | 70–86 s | 265–278 s | 2.4 s | 12.1 s |

- With the counts in the chunks, they count all 31,210 documents and none is walked in stats.
- The every-row count (item 3) took the stats phase from 8.5 to 2.4 s.
- Item 1 moves the making off the evaluation: the chunks' joins make no document (0.04 s). The worker that runs ahead
  makes 41.6 s, 5 s more than the evaluation made, because it makes documents no read asked for.
- The cross part went from 5.8 to 12.1 s: the corpus batch's documents, which the stats phase projected before, are
  now projected in cross (6.2 s). That is the corpus batch, read by the cross part, not the chunks' blocks.

Against 0431 step 7 the run still retires 9% more instructions and peaks at 2.4 times its memory.

The executor coverage test (`every_operation_runs_on_every_executor`) fails on the Linux hosts' CUDA device with
error 700 (an illegal memory access), with and without this change; it passed once without it.
