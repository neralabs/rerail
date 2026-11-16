# 0264: The atom arena is read without a lock (RCU over an append-only arena)

Status: draft, not built (2026-11-15): §0's probe bounds its gain at about 0.4 s of a 40 s run, below the
cost of the rewrite for now. Refines the arena of decisions 0049 and 0111 (`content_store::atoms`). Keeps the
reservations of 0114 §4, the provisional ids of 0160 and the fork map of 0170. Follows decision 0263, whose
§4 keeps text-reading joins on one thread because of this lock.

## Intro

Every read of the atom arena takes one `RwLock<State>`: a text by its id (`with_str`), an id by its text (`get`,
`existing`, `lookup`). A shared read lock is still a write of one word that every reader shares. A join that
reads texts row by row takes it about four times a row, and a miss takes it again in `existing` and `lookup`.

On all of PostHog (an M3 Max, 16 threads), `exports/resolve_export` walked in morsels took 2.7 s against 1.0 s on
one thread, for the same work. A stack sample of that window found every thread in `lock_shared_slow`, under
`Atoms::existing` and `Atoms::lookup` (0263 §4).

The readers cannot simply skip the lock, because the writers move what the readers read:

- `intern_all` appends to `buf`, `ends`, `hashes` and the hash table, and so reallocates them.
- `store_upto`, run by the store writer (1,794 times on PostHog), moves the stored atoms' bytes into a held
  segment (`split_off`, `shrink_to_fit`) and renumbers the pending ends.
- Reading a segment from the database (`read_atoms`, `add_stored`) inserts into the `held` map.

Writers come in batches: about 3,500 `intern atoms` batches while the chunks project, beside the chunks' readers.

MLDB's answer (`arch/gc_lock.h`, `arch/rcu_protected.h`):

- Readers enter an epoch, and writers publish a new version by swapping a pointer (`RcuProtected::replace`).
- What a reader may still hold is freed only once every reader has left the epoch it was read in
  (`deferDelete`).
- A reader enters per many reads, not per read (`lockSpeculative`, `SpeculativeThreshold`).

## Decision

### 0. Measured first: what lock-free reads would gain

The probe changes the arena's lock to a reader-sharded lock (`crossbeam_utils::ShardedLock`): each reader writes
only its own shard's word, and a writer takes them all. The arena's data is unchanged, so writers still block
readers and `store_upto` still copies. Joins that read texts are let into morsels for the probe
(`RERAIL_PROBE_TEXT_MORSELS=1`).

Results: see Measured. The probe's patch is kept outside the tree (`working-data/probes/0264-sharded-arena-lock.patch`,
git-ignored).

### 1. The arena's memory never moves

- **Atoms by id.** Bytes and ends sit in blocks of fixed size, which are allocated once and never reallocated. An
  atom's place is (block, offset), and a reader reads it in place.
- **Pending and stored.** These become a watermark (`base`, atomic), not a move. `store_upto` advances the
  watermark and copies nothing.
- **Stored segments.** Segments read from the database (0111) are held in a directory of segment pointers, by
  segment number. A segment is published whole, then its pointer is stored with release ordering.

### 2. An id by its text: atomic slots, the table swapped when it grows

The table is open-addressed. Each slot is one `AtomicU64` holding the tag (the hash's high half, as now) and the
id.

- Readers probe it with acquire loads and no lock.
- One writer, holding the writer's mutex, fills an empty slot with a release store once the atom's bytes are
  published. So a reader that finds the slot finds the text.
- Growing the table builds a new one, copies the slots into it, and swaps the table's pointer
  (`RcuProtected::replace`). The old table is freed once no reader can still hold it.

### 3. Readers pin an epoch per stretch of reads

Reclamation is by epoch: `crossbeam-epoch` (already in the tree, through rayon), or a small `GcLock` of our own.
Two things can be retired: old tables (§2) and old directories (§1).

A reader pins once per stretch of reads, as in MLDB's speculative section, not once per read:

- a walk in morsels, per morsel;
- a chunk's projection, per batch;
- any other read, per call.

A pin is a write to the thread's own word. The readers share none.

### 4. What stays under a lock

These are the rare paths, and they stay as they are:

- the lease and `catch_up` (0049);
- the database reader (`find`, `read_atoms`);
- a fork's map (0170);
- the reservations (0114 §4: thread-local, nesting as in 0263 §5);
- writes, under one writer's mutex.

"A miss is final" (`misses_are_final`) stays atomic loads.

### 5. Then the text joins walk in morsels

0263 §4's exclusion goes. The text steps then cost each morsel a pin, not a lock a row.

## Measured

All of PostHog, cold store, profiling build, an M3 Max, K = 15, unsampled. Each configuration ran twice,
alternating (load average 10–30 from other work). The committed engine of 0263 is the base. Every run gave the
same 29,747 violations (483a6f116876), with no panic.

| | `resolve_export` | `sym_of` | `sym_of__sup0` | cross part | chunk phase | wall |
|---|---|---|---|---|---|---|
| base (0263) | 0.92 / 1.01 s | 0.32 / 0.32 s | 0.77 / 0.74 s | 4.83 / 4.78 s | 23.9 / 23.1 s | 42.9 / 41.3 s |
| sharded lock, text joins in morsels | 0.77 / 0.74 s | 0.25 / 0.27 s | 0.72 / 0.73 s | 4.41 / 4.45 s | 22.7 / 22.7 s | 42.3 / 40.6 s |
| sharded lock, text joins whole | 1.07 / 1.00 s | 0.30 / 0.33 s | 0.76 / 0.72 s | 4.79 / 4.79 s | 22.4 / 24.0 s | 40.2 / 42.8 s |

What this shows:

- **Without the lock's shared word, text joins in morsels stop losing.** `resolve_export` falls from 2.7 s in
  morsels under the current lock to 0.75 s, against 0.97 s on one thread.
- **That is only a quarter faster than one thread, on 16 threads.** Something other than the arena's lock bounds
  it. §6 traces what: not the waves' serial parts, as first guessed.
- **The chunk phase does not move** (22.4–24.0 s in all three). The arena's lock is not what holds its threads
  at 6–7 of 16 busy.
- **The whole gain within reach is about 0.4 s** of the cross part, about 1% of the run. A sharded lock already
  gets it. An RCU arena would at most add the readers' last shared writes, which the probe already removed, and
  stop `store_upto`'s copies.

### 6. Traced: what bounds `resolve_export` on one thread

The wave trace (`RDL_WAVE_TRACE`) of a full PostHog run on the M3 times each probe of the stratum's 13 runs (the
whole plan's budgeted pieces, then the waves):

| probe | runs | wall | largest |
|---|---|---|---|
| 2: `resolve_export($t, $q), $t = @cat($m2, $x), fl272_exports(…)` | 13 | about 700 ms | 256 ms: 134,636 delta rows, 447,804 parts, 126,558 rows made |
| 4: `resolve_export(@cat($m, $x), $r) :- exports(…)` | 9 | about 190 ms | 61 ms: 128,903 rows, each `@cat` interned |
| the others | | about 15 ms | |

That is about 0.9 s of the 0.97 s stratum, and the walks' thread CPU (`native.tsv`) is 0.70 s and 0.18 s of it. So
the waves' serial parts (settling, indexing, replanning) are not the bound. Probe 2's walk is.

Probe 2 splits each delta text at every "." and looks up both parts by text: about 1.09 M parts, about 0.57 µs each
on one core. A stack sample of the stratum puts the walk's self time in `Interner::get`. The table holds millions
of atoms, so each probe is a random cache miss, and a hit then reads the candidate's string (another miss) to
compare. That is memory latency per part, not locks: one thread is bound by it, and threads would share it out.

Two overheads on that path are removed (2026-11-15):

- **A stored atom's segment was found in a `BTreeMap`** of the ~1,800 segments a run holds (`store_upto` adds one
  per write). Every text read of a stored atom, and every hash hit's compare, walked it. The held segments are now
  a sorted array of starts beside the segments (`Segments`), searched in place.
- **A split read the arena once per text:** the whole, then each part. It now reads the whole and its parts'
  atoms under one read (`Atoms::in_memory`), and a part not in memory is no atom where misses are final.
  `Atoms::existing` ends at `misses_are_final` too, not at `lookup`.

| M3 Max, all of PostHog, alternating (load average 16–42) | probe 2's CPU | `resolve_export` | cross part |
|---|---|---|---|
| base (0263) | 0.78 / 0.80 s | 0.88 / 1.01 s | 4.78 / 4.86 s |
| §6 | 0.69 / 0.66 s | 0.92 / 0.86 s | 4.66 / 4.54 s |

All four runs gave the same 29,747 violations (483a6f116876). What remains is the lookups' memory latency. The
levers left are threads (§5, after the lock) and fewer lookups (a plan that reads the parts' atoms by id).

## Open

- **The unit of reclamation.** Per table and per directory are proposed here. Per block is not needed, since
  blocks never move.
- **The text steps a chunk at a time** (one `lookup_all` per chunk, as row programs read atoms). This is
  independent of the lock, and helps one thread too.
- **Fewer text lookups in a split.** Probe 2 looks up about 2.4 parts per delta row, and most miss. A plan that
  starts from the parts' relation, or a key rule that keeps each qualified name's parts by id (decision 0103's
  cache form), would read ids, not texts.
- **What holds the chunk phase at 6–7 busy threads:** not this lock (Measured).
- **Taking the sharded lock alone** (the probe's patch, cleaned up) would keep most of §0's 0.4 s at a few
  lines' cost, with 0263 §4 lifted. Writers then take every shard. The probe showed no writer cost, but it was
  not measured apart.
