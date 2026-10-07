# Content store bulk publication design

## Purpose

This design removes long, Rust-mediated SQLite write chains from file publication.
It permits concurrent reads and commits valid partial work before a correction pass.

The design has these required properties:

- Each worker thread owns one SQLite connection.
- Read transactions can overlap a write transaction in WAL mode.
- Each lookup uses one bulk query with CTEs and joins.
- Source and tape state comes from one joined lookup.
- Hash-prefix indexes contain only the algorithm and first signed hash lane.
- SQL compares the remaining lanes for identity safety.
- Rust completes compression and serialization before a write transaction.
- A stale read causes a committed correction pass, not loss of valid work.
- Each reconciliation pass executes at most ten data statements.
- A completed-revision marker is the visibility boundary.

## Connection ownership

`ContentStore` stores the database path, compression settings, and object caches.
It does not store a SQLite connection.

A thread-local map owns one connection for each database path used by that thread.
The maintenance thread, Rayon workers, and N-API task threads therefore use different connections.
Each connection has the same WAL, foreign-key, timeout, cache, and statement-cache settings.

Schema migration uses a short, separate connection during `ContentStore::open`.
The connection closes after migration finishes.

## Hash lookup

A hash identity contains an algorithm and four signed 64-bit lanes.
Lookup indexes use only the algorithm and first lane:

```sql
CREATE INDEX source_hash_prefix
ON source_objects(hash_algorithm_id, hash_lane0);
```

A lookup uses the prefix index and checks all remaining lanes:

```sql
... ON object.hash_algorithm_id = input.hash_algorithm_id
   AND object.hash_lane0 = input.hash_lane0
   AND object.hash_lane1 = input.hash_lane1
   AND object.hash_lane2 = input.hash_lane2
   AND object.hash_lane3 = input.hash_lane3
```

The first lane selects an extremely small collision bucket.
The remaining comparisons protect identity correctness.

The writer serializes insertion with `BEGIN IMMEDIATE`.
It checks the complete identity before it inserts a row.
A full-width uniqueness index is not necessary for lookup or convergence.

## Reconciliation input

One publication contains at most 64 files.
Rust creates one immutable input model for the complete publication.

The input contains:

- Paths and path hashes.
- Canonical source hashes and Git aliases.
- Compressed source frames for missing sources.
- Tape grammar identities, hashes, and compressed frames.
- Candidate catalog and check identities.
- Source and candidate anchor facts.
- Compressed local-result frames.
- Expected overlay and relationship rows.

JSON parameters carry variable-size integer and text identities.
Bound BLOB parameters carry compressed source, tape, and result frames.
Rust does not convert BLOBs to JSON or base64.

## Read phase

Each reconciliation pass starts with a short deferred transaction:

```text
BEGIN
bulk state lookup
COMMIT
```

The lookup uses one input CTE and joined projections.
It returns source, alias, tape, path, catalog, check, overlay, candidate, and result state.
Source and tape lookup occur in the same statement.

The transaction closes before hashing, compression, or serialization starts.

## Preparation phase

Rust compares the bulk result with the immutable publication input.
It then prepares only missing or incorrect work.

Rust completes these operations without a SQLite transaction:

- Full identity validation.
- Source and tape hashing.
- Zstandard compression.
- Result-frame serialization.
- JSON encoding for all relationship inputs.
- BLOB and scalar parameter construction.
- Expected-row count calculation.

A correction pass reuses immutable frames from an earlier pass.
It does not recompress an object that another writer inserted.

## Write phase

A write pass starts with `BEGIN IMMEDIATE`.
Each statement consumes the complete prepared input and uses joins to resolve dependencies.
Rust does not fetch an ID and issue a dependent statement for each row.

An object pass can execute these statements:

1. Insert missing hash identities.
2. Insert missing strings.
3. Insert missing paths.
4. Insert missing candidate catalogs.
5. Insert missing candidate checks.
6. Insert missing sources.
7. Insert or verify Git aliases.
8. Insert or verify tapes.
9. Insert source anchor facts.

The transaction commits valid rows even when a dependency changed after the read phase.

A relationship pass can execute these statements:

1. Upsert workspace overlays.
2. Insert candidate anchor facts.
3. Replace candidate-completion markers.
4. Replace candidate file relationships.
5. Insert local results.
6. Write the completed-revision marker when no work remains.

Each pass stays below the ten-statement limit.
Transaction-control statements are measured separately.

## Stale snapshots and correction passes

A prepared statement never trusts an integer ID from the read phase without checking its identity.
It joins the current database rows by complete identity inside the write transaction.

If another writer changes the state, the current pass does this:

1. It inserts or updates every row that is still valid.
2. It skips each row whose dependency is not available.
3. It commits the valid rows.
4. It starts another bulk read transaction.
5. It prepares only the remaining work.
6. It executes another correction pass.

The loop stops when all expected rows exist with correct identities.
A fatal identity conflict stops completion but does not remove valid committed objects.

The loop requires monotonic progress.
A pass reports inserted, reused, corrected, deferred, and remaining row counts.
A no-progress pass rereads after bounded busy backoff.

## Visibility and crash recovery

Intermediate object and relationship rows are reusable but incomplete.
Readers use the completed-revision marker as the publication boundary.

A process can exit after any committed pass.
The next process reads the incomplete state and publishes only missing rows.
It writes the completion marker only after the final bulk lookup proves completion.

Cancellation stops the loop between transactions.
Committed work remains available for a later correction pass.
An uncommitted transaction rolls back.

## Statement budget

A publication pass owns a statement counter.
Every prepared-statement execution increments the counter.
The pass fails before commit if the data-statement count exceeds ten.

Tests assert the limit for:

- A fully cold 64-file batch.
- A fully warm 64-file batch.
- A first-writer race.
- A stale-read correction pass.
- Empty candidate and result sets.
- Maximum candidate and result sets.

Metrics report:

- Read statements.
- Write statements.
- Transaction-control statements.
- Reconciliation passes.
- Committed rows.
- Deferred rows.
- Remaining rows.
- Read time.
- Preparation time.
- Writer-lock time.
- Commit time.
- Busy wait time.

## Baseline

The pre-redesign baseline and the runtime journal's final comparison are preserved at commit `83f3eae6e`:

- `benchmark/history/ner-1562-publication-transaction-baseline-2026-09-25.json`
- `docs/workspace-store-runtime-journal.md`
