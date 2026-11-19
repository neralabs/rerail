# 0296. The cross part's tail off the critical path: its kept part framed in jobs, the file-local part's writes sealed before the cross part, and its new atoms written while its kept part is framed; collection keeps a watched root's answers

Status: decided (2026-11-16). Builds on 0294 (the cross part's strata off one thread), 0102 and 0104 (writes behind
the run, in waves), 0049 §4.3 (the run's atoms at its end), 0117 and 0212 (the kept part's frames, digests of
content), 0053 (collection after the manifest).

## Intro

After 0294, all of PostHog on the M3 Max ran its cross part in 1.3 s, then about 3 s on one core to the end:

| from (s) | what | time |
|---|---|---|
| 29.8 | `retain`: the kept part's changes framed (`Retained::write_frames`), on the main thread | 0.73 s |
| 30.6 | the writer: the files' stats, then the frames (207 MB) | 0.29 + 0.39 s |
| 31.0 | the writer: the batch's new atoms (388,099 the cross part made, `@cat`), its commit | 0.6 s |
| 31.6 | the run's manifest, then the store's collection, which the run waits for | 0.4 s |

A stack sample of that window showed why the writer started only at 30.6: the jobs a run queues join its wave,
which only a boundary sends to the writer (0104). The files' stats are queued when the file-local part ends, but the
next boundary was the cross part's end (`Boundary::Cross`), after `retain`: the writer sat idle through the cross
part. And `write_frames` framed each input on the main thread, one after another, and each whole relation in one
job (`sym_of`'s 6.4 M rows digested and encoded on one thread).

## Decision

### 1. The kept part is framed in jobs

- Each input's frames (its documents' digests, its changed granules' rows, each encoded) are a job of their own
  ("an input's frames (job)"); their frames are taken in the inputs' order, as before.
- `Cells::sum` (a relation's digest, `rows_digest`) sums many rows in parts at once: a wrapping sum, the same in any
  order.
- `cross_rows::encode` encodes many rows in parts at once: each column's rows in parts, each part's first value
  the difference from the row above it, so the parts' bytes one after another are one thread's.
  `cross_rows::tests::a_block_encoded_in_parts_is_one_threads` checks it.

### 2. The file-local part's writes are sealed before the cross part

A new boundary, `Boundary::Local`, seals the run's wave before the cross part: what the file-local part left queued
(the files' stats) is written while the cross part runs. A wave is still written whole; the file-local part's last
writes are one more wave, as a chunk's are.

### 3. The cross part's new atoms are written while its kept part is framed

Once the cross part's rows are derived, its pending atoms are queued (`queue_atoms_flush`) and sealed in a wave of
their own (`Boundary::CrossDerived`, `corpus::cross_atoms_ahead`): the writer appends them while `retain` frames the
kept part, not after the frames in the cross part's last wave. A warm run makes no atom: nothing is queued (checked
with `RERAIL_WARM=list`: no pending-atoms or files'-stats write in a warm run).

### 4. Collection keeps what a run has not settled yet (superseded by 0250)

`observe::collect` dropped an answer, a canonical form or a Git file list under no root whose cursor the observer's
store holds. A root first asked in a run has its cursor stored at the outermost run's end (`settle_roots`), but its
answers are written at each inner run's end (phase 1's), and the store's collection runs between the two, after the
manifest. So on a cold store, whose first run always collects, every answer and canonical form under the corpus
root was dropped (PostHog: 38,940 answers, 29,772 records; `RERAIL_OBSERVE_TRACE` lists the roots collect saw, the
corpus's not among them), and the next, warm run read and recorded each again (2.8 s, against 1.2 s).

This branch kept what is under the roots this process watched (63a4c75c5). Decision 0250, on the hub at the same
time, keeps what was recorded within the last hour (`UNHELD_GRACE_NS`): it covers a collection in another process
too, so the merge of the hub (2026-11-16) took it, and dropped `observe::watched`. The trace stays.

### 5. A profiled warm run makes its profile's directory (superseded by the hub)

A run the whole-run cutoff answers wrote no report, so nothing made `RERAIL_PROFILE`'s directory, and the run's
lifecycle line failed to open: the run failed after writing its outputs. The hub makes the directory and writes the
run's spans as its report; the merge took that.

## Measured

All of PostHog, profiling build, the M3 Max (load about 3 at the start), the same 29,747 findings (the same set,
node ids included, fa0efeaed6ff) in every run:

| | 0294 | 0296 |
|---|---|---|
| wall | 34.5 s, 33.1 s | 34.3 s, 32.6 s |
| `retain` on the main thread | 0.73 s | 0.11 s |
| the cross part's evaluation done to the manifest written | 1.82 s | 1.00 s |

A cold run, then a warm run on the same store and outputs (`reuse: true`, the command line's), all of PostHog,
unprofiled, the M3 Max, the same findings in each:

| | cold | warm | the warm run's observer |
|---|---|---|---|
| before §4 | 30.7 s | 2.83 s | every corpus answer recorded again |
| after §4 | 31.0 s, 30.8 s | 1.21 s, 1.44 s | 38,956 answers read, none recorded |

After the hub's merge (4b0758b56, 2026-11-16; the M3 shared with other builds, load 5-13), the same findings in
every run:

| | cold, profiled (`reuse: false`) | cold, unprofiled | warm, unprofiled |
|---|---|---|---|
| wall | 33.9 s, 33.2 s | 33.6 s, 31.2 s | 1.08 s, 1.09 s (no warm write) |
| `retain` on the main thread | 0.11 s | | |
| the cross part's evaluation done to the manifest written | 0.84 s | | |

The writer's order in the tail is unchanged: the cross part's atoms (0.39 s, beside `retain`), the frames (0.37 s),
the commit, the manifest, then collection.

## Open

- The writer is the tail's critical path now: the cross part's 388 k atoms (0.55 s, mostly their keys, `atom_key`,
  a B-tree insert each), then the frames (207 MB of blobs, 0.37 s: `sym_of` 39 MB, `sym_of__sup0` 33 MB,
  `export_query__sup0` 15 MB, ...), then the manifest. Writing the keys later or in another form, and keeping
  fewer of the kept part's relations (a supplementary relation is kept for maintenance), are tradeoffs of the
  store's format and of maintenance.
- The store's collection runs after the manifest and the run waits for it (0.34 s on a cold store, whose first run
  always decides).
- (Closed by the hub's 0284, rerail's own files outside the observer: the warm run writes nothing.) A warm run
  wrote the observer's store once: phase 0's outputs, a directory of each process's under the temporary directory.
- A profiled run's report (`profile::report::write`, the roofline's file digests) takes about 0.4 s of the end;
  an unprofiled run has none.
