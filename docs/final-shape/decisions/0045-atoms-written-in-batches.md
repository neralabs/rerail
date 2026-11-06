# 0045: Atoms are allocated in memory and written in a few large commits

Status: proposed (2026-11-06)

## Intro

Every name, path and string in the engine is an atom: a dense integer id, stored in the `atoms`
table (`atom`, `value`) with a unique index by value (`atoms_by_value`). Ids are given out in
order. A process keeps a copy of the table in memory (a mirror), so it can find an atom without a
query.

A corpus run builds its files in chunks. Each chunk's new strings are written to `atoms` in a
transaction of their own, and committed, before the chunk's rules run. On the 9,010-file corpus
that is 104 commits for 640k new strings. Each commit rewrites about 1,900 index pages, because
the new strings are spread all over the index. That is about 200k pages, roughly 800 MB, written
per run (open item O3; notes T10 §1b). The volume follows the number of commits, not the number
of strings.

The per-chunk commit is not needed for correctness. It exists because the process has **two**
mirrors that only meet through the table:

- the store's (`content_store::atoms::Interner`), which interns the chunk's strings on the batch
  worker thread;
- the planner's symbols (`logic::native::setfix::Interner`), which the rules' SQL functions
  (`sym_text`, `sym_atom`, `sym_cat`) read and extend on the main thread.

The planner's mirror learns the chunk's atoms by reading the table's tail. So they must be in the
table first. And because both mirrors hand out ids, a thread-level lock (`atoms::hold`) makes a
planner run the only writer while it runs, so the worker's next chunk waits for it.

This record proposes one mirror per database in a process, with ids allocated in memory and the
rows written later, in a few large commits.

## Decision

### 1. One mirror per database, shared

A process holds one `Interner` per database file, found by canonical path (as `atoms::hold`
finds its lock today). The store and the planner's symbols both use it, behind one reader–writer
lock. An id is given out under the write lock, so the two can never give out the same id. This
also removes the need for `hold`, and the worker no longer waits for the planner run.

The mirror keeps a watermark, `stored`: ids below it are in the table, and ids from it on are
**pending**.

```text
// Ids 0 .. stored are in the table; stored .. len are pending (in memory only).
struct Shared { atoms: Interner, stored: usize, lease: Option<Lease> }
```

A failed planner run's symbols are no longer truncated away. Strings in `atoms` that nothing refers
to are harmless, since the table is append-only.

### 2. Nothing durable refers to a pending atom

Pending atoms are written (**flushed**) to the table before anything that could make them visible
outside the mirror:

1. **In every write transaction on the database, first.** All of the store's writes go through
   `immediate_transaction`, `immediate_transaction_once` and `immediate_transaction_no_wait`. Each
   one flushes pending atoms at the start of its transaction. So the atoms and the rows that refer
   to them commit together, in one commit, not two. A planner run's `flush_symbols` becomes this
   same flush.
2. **Before SQL that reads `atoms`.** The vocabulary fill (`prefilter/vocabulary-fill.sql`), the
   prefilter's term lookups (`gates.rs`, `access.rs`, `sets.rs`), `text/vocabulary.rs`,
   `stats/inputs.rs` and the derivations' joins all join `atoms` by value or id. Each one calls the
   flush first. A debug assertion checks this: in test builds, a connection with pending atoms that
   prepares a statement reading `atoms`, other than the flush, fails.
3. **When enough is pending:** 256k atoms, or 16 MB of strings. This bounds the work lost in a
   crash and the size of one flush's WAL. On the full corpus that is about 3 commits instead of
   104 (A).
4. **At the end of a run, and when the store is closed.**
5. **When another process asks for the lease** (§4).

The prefilter's `Counts::flush` stops inserting into `atoms` itself (`INSERT … ON CONFLICT DO
NOTHING`, with ids taken by SQLite). It interns its terms through the mirror like every other
writer. It runs inside a store transaction, so the flush in point 1 covers it.

The corpus run's `cross.sqlite` is the run's scratch database, recreated at the start of every
run. Its rows don't count as durable references.

### 3. A flush writes in value order

A flush inserts the pending rows sorted by value, each with its id. The ids are unchanged, so the
order doesn't matter to anything else. The table's rows are appended in id order either way. The
index then fills leaf by leaf: each leaf is written once per flush, not once per string. The flush
keeps the bulk settings already in place (a 32 MB cache and a 16k-page checkpoint interval, for the
transaction only).

The expected volume is about the new rows' and index entries' own size: 640k strings at about 30
bytes is about 5–10k pages per run, against 200k (A).

### 4. Other processes: a lease

Pending ids are safe only while no other process appends to `atoms`. The rule is a lease: a single
row, `atom_lease(holder, pid, heartbeat, wanted)`.

- A process takes the lease, in a short transaction, before it gives out its first pending id. It
  gives the lease back at its flush, or keeps it and refreshes the heartbeat while it still has
  work.
- A process that wants to append and finds another holder sets `wanted`, then waits, up to its busy
  timeout, the same as for SQLite's write lock.
- The holder checks `wanted` at each chunk boundary, with one read. If it is set, the holder
  flushes and releases the lease.
- A lease whose holder's process is gone, or whose heartbeat is stale, can be taken over. That is
  safe: by §2, nothing durable refers to the old holder's pending ids, so they can be given out
  again.

On open, a process rebuilds its mirror from the table. Pending ids never outlive the process.

## What the prior work says

- **Process-held state.** The spike lost planning state that was held only in Python, so a
  reopened store planned differently. Its rule is that process-held state (interners, frequencies)
  is rebuilt on attach (history index, DH L3319–3343, S174/S178). Pending atoms follow that rule:
  they are only ever process-held, and nothing durable depends on them.
- **Ids that depend on order.** In the spike, stored bytes depended on batch order because kinds
  were interned at first sight (history index, S130; design index: "interning on first observation
  made the bytes order-dependent"). Kinds and fields here are grammar codes, not atoms (decision
  0009). Name atoms already depend on the order of chunks and runs. This record changes when
  they're written, not which ids they get.
- **Decision 0009.** Symbols are atoms, and strings exist only in `sym_text`, `sym_atom` and
  `sym_cat` over the mirror. The shared mirror keeps that, with one mirror instead of two.
- **Decision 0024 §6.** It sets 1 MB of page cache per connection at rest. The flush's larger
  cache is for its transaction only, as it is now.
- **Open item O3's closure note.** It proposed "intern ahead … with a reserved range and write the
  atoms with the first layer that uses them". §2.1 is that rule. The lease replaces a reserved
  range, because the table stays dense.

## Order of work

1. Point the store and the planner's symbols at one shared mirror, and remove `hold`. Test: the
   worker and a planner run intern at the same time and get distinct ids.
2. Add the watermark and the flush, and call the flush in the three transaction helpers, the
   reader sites and the end of a run. Change `Counts::flush`. Test: a write transaction after
   pending atoms commits them with its rows; a reader site sees pending atoms.
3. Sort the flush by value. Test: ids are unchanged, and the pages written are counted.
4. Add the lease, and its takeover from a dead holder. Test: two connections acting as two
   processes.
5. Measure on the full corpus: commits, pages written, CPU and instructions retired for the batch
   phase. The replay's totals must stay identical.

## Expected effect

| | Now | Expected (A) |
|---|---|---|
| Commits of atoms per full-corpus run | 104 | about 3 |
| Pages written | about 200k (800 MB) | about 5–10k |
| Interning's share of the batch phase | 44% | under 5% |
| Worker waits for the planner run | yes (`hold`) | no |
