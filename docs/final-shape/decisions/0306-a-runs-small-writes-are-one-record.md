# 0306. A run's small writes are one record, applied at its end

Status: decided (2026-11-16), by the user ("can we use a really simple encoding for small records like JSON and just
jam them in a single string at the end of the data flow?"; "go ahead. We may want to change JSON for something
else"). Building. Builds 0259's guard (no queued write is a closure) for the small writes; replaces the typed-rows
proposal for them (P1–P6, below). Keeps 0295's segments for the bulk writes, and 0104 (no SQL inside the waves).

## Intro

A run hands two kinds of write to the store's writer. **Bulk writes** (a file's tape and source, its frames, its
memo entry) are most of the bytes, and the run reads them back while they are queued: they are in 0295's mapped
segments already, each a pointer row once archived. **Small writes** are everything else: the files' stats, the
manifest, the pins and holds, the kept plans, statistics and choices, the programs a run prepared. Until now each
was a closure queued on the writer, holding whatever it captured (sources found later, the prepared program, the
run's collection lease), which a crash, a fork or another process cannot replay, and which 0259's guard forbids.

This decision makes the small writes one value per run: a **record**, a list of items, each a kind and its data,
encoded as one string at the run's end and applied by the writer in order.

## Measured (2026-11-16)

corpus0's first 1, 10 and 50 files, 312 of the bench's 317 rules (the five Q29 stops left out), strict, cold, warm and
a one-file edit (`scratchpad/wal/run.sh`, not kept):

| 50 files | Cold | Warm | Edit |
|---|---|---|---|
| Jobs queued (1 / 10 / 50 files) | 103 / 110 / 114 | 0 | 33 |
| Writer CPU | 1.0 s (about 1% of the run) | 0 | 0.06 s |
| Rows written | about 190,000 | 0 | about 1,800 |
| Bytes in segments (memo 97 MB, frames 66 MB, objects 5.5 MB) | 169 MB | 0 | small |

- **Who reads what, when.** Only the bulk kinds are read while queued (`pending`: a queued tape, a frame, a memo
  entry). No small write is: each reader waits for the writer (`settle`) and reads SQLite (the stat index at a run's
  start, the planner's choices, a prepared or combined program), or is a later process (the next run, collection).
- **Small writes are small and few.** About 2,000 rows a cold run; jobs do not grow with the corpus (one or two a
  kind a run).
- **Every reference between them is by content already.** A source is found by its hash on the writer
  (`Content::resolve`, `memo::source`); none names another job's row.
- **What they cost.** The kinds that compute rather than store (the prepared program 0.22 s, the planning choices
  0.17 s, the combined program 0.09 s, a cold run's; none on an edit) encode programs into facts on the writer.

## Decision

1. **A record per run.** A small write is an item of the run's record: a **kind** (a name) and its **body**
   (owned data), with the keys a `queued` or `settle` asks about. Items are added in the run's order. Outside a run
   (no waves), an item is a record of its own, applied at once behind the caller.
2. **One string, at the end of the data flow.** At the run's end (its `Waves` dropped, `Boundary::End`) the record
   is encoded as **one string** and queued as the last job of the run's last wave, after its bulk writes. Binary
   bodies (a program's image, a blob) are not put in the string: the record has **one byte area** beside it, and an
   item names its range. A `settle` of a key the record holds, or `written`, closes the record early; later items
   start the next.
3. **Applied in order, by registered kind.** The writer decodes the string and applies the items in order, in the
   wave's transaction. Each kind registers its apply function under its name (the store's kinds in the store, the
   logic's in the logic): the queue holds data, the registry holds code. An apply is given the store and the
   record's **named sources**: each item that stores or finds a source adds it, and the manifest's item, last, names
   them. This replaces the closures' shared `late` list.
4. **Bodies are data.** No body holds a handle, a lease, a lock or an `Arc` of the run's: what a closure computed from
   such a thing (the prepared program's facts, a rewritten program's text) is computed on the run's thread and the
   result put in the body (about 0.5 s a cold run, 1%; none on warm or edit runs). Store ids are allowed only for
   sources the run read from this store (held by its collection lease); others are named by digest.
5. **What follows a commit stays on the run's side.** The manifest's item records the manifest; releasing the run's
   collection lease and starting collection, which a closure did after it, are done by the run once the record is
   written (`settle` of the manifest's key).
6. **The encoding is one module's** (`record::codec`): JSON now (`serde_json`). Changing it (CBOR, a packed form)
   changes that module only; a record carries its format's version, and a record of another version is not
   applied (the next run computes what it held again).
7. **Not durable yet.** The string is the job's payload, in memory. Writing it into the run's last segment, so a
   dead process's record is applied by the next, waits for the bulk segments being replayable (0295 §3 removes a
   dead process's segments today): a manifest applied without its sources' segments would name missing sources.
8. **The guard.** With the small writes records and the bulk and store-internal writes (frames, memo, objects,
   atoms, a blob) jobs of a closed set of data variants, `write_back` takes no closure, so none can be queued. A
   test-only variant blocks the writer for the queue's tests.

## Rejected

- **Typed rows in segments for every kind (P1–P6, 2026-11-16).** Declared write relations, a digest dictionary per
  block, SQL applies over a virtual table, one block format with facts, replay of a dead process's segments. Sound,
  but built for volumes the small writes do not have (2,000 rows, 1% of the writer), and tied 0259's guard to the
  virtual-table module.
- **A closed enum of store operations for the small writes.** Simpler to apply, but every chain of lookups a
  closure made (find the combined program's text, name it if stored) becomes a store operation, and the logic's
  kinds move into the store crate.

## Path

1. The record, its codec and the registry; `settle`, `queued` and `written` see its keys. Test: a record's items
   are applied in order at the run's end, and a `settle` of an item's key applies it first.
2. The small kinds, one at a time, each with its targeted tests: the files' stats, the pins and holds, the kept
   statistics, plans and choices, the programs, the manifest (with 5) last.
3. The bulk and store-internal jobs as data variants; `Job` (the closure) removed.
4. The run-start holds: a running run holds the shared collection lease, which collectors skip, so the holds may be
   redundant. A test that collection never takes a run's plan or source while the run lives, without them, decides
   whether they go.
