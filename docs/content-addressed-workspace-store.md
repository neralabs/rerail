# Content-addressed workspace store

## Status

This document defines the proposed source and syntax-tape store for the Rerail daemon.
It records the design evidence, storage model, ownership rules, and garbage-collection model.

The design supports approximately 100 concurrent workspaces with mostly identical source code.
The design has these machine-wide limits:

- steady-state Rerail RSS of less than 1 GiB.
- total store size of less than 10 GB.
- repository audits stop after 100,000 files or 512 MiB of source; a repository index holds
  at most one batch of source (32 files, reading stops at 1 MiB, plus at most one 32 MB file).
- interactive edit p95 latency of less than 500 ms.

This design does not change the session database schema or the rule-ownership model.
It adds a machine-wide content-addressed store for source, tapes, snapshots, and overlays.
All database operations must move behind the Rust daemon boundary.

## Required boundary

JavaScript must never access the database.

Rust owns all SQLite connections, SQL statements, transactions, migrations, caches, and garbage collection.
JavaScript must not import an SQLite client or construct SQL.
JavaScript must not receive table names, row identifiers, hash lanes, or database errors.

The JavaScript daemon can call coarse native storage operations.
These operations form an internal daemon interface, not a database interface.
The Rust implementation returns domain results and bounded byte buffers.

The Rust implementation runs blocking database work outside the JavaScript event loop.
Background garbage collection also runs in Rust.

Repository-scope audits of a Git checkout run `indexRepositoryAsync()` on the audit workspace,
one bounded chunk per call. Rust walks the workspace root exactly as the JavaScript walk does
(workspace-root-relative regular files with the host's source languages and code-rule scopes, never
through a symlink, outside the host's ignored directories, and, when the repository follows `.gitignore` (the default),
outside what Git ignores); `.rerailignore` and the built-in ignores decide only which clean files it may restore, never
which files it evaluates.
It restores every completed clean directory from the store, then reads one batch per chunk from
the worktree without following symlinks, publishes it and releases its sources and tapes before
the chunk returns. Listing and restoring evaluate nothing, so they run without the process-wide
evaluation slot; only a chunk's read and apply take it. JavaScript transfers no repository
source; between chunks it lets interactive edits run first and applies the 100,000-file, 512 MiB,
20-failure and five-minute bounds. A complete index records its clean trees for reuse through
`completeStorageRevisionAsync()` on a libuv pool thread; that write is a cache, so a busy or
failing store costs the reuse, never the audit's results. Outside a Git checkout the audit walks
the filesystem in JavaScript under the same bounds.
`applyBufferAsync()` remains available for unsaved and generated content.

Publication stores a file under its committed Git blob alias whenever its evaluated bytes equal
that blob, whoever supplied them (repository index or host edit). It hashes only files whose path
is clean in the worktree, and a clean file is promoted into the snapshot.

Schema 9 records completed clean Git trees.
Each record includes the tree hash, logical path prefix, snapshot, and semantic catalog identity.
A tree is not eligible if Git reports an overlay below its prefix.
The runtime restores only maximal matching prefixes.
Schema 10 stores result pages for each clean Git tree and logical prefix.
Each page contains at most 64 direct file paths, violations, and deterministic failures.
The store compresses each page in an independent level-3 Zstandard frame.

A matching subtree attaches all of its result segments without per-file restoration.
The runtime does not create file state, load sources, hash sources, restore tapes, or decode local check results.
Result paging merges attached violations with dirty overlays in deterministic result order.
Code rules with global clauses continue to use per-file state because they require complete fact evidence.

Repository restoration resolves file identities and saved results in bulk.
Rust resolves clean Git aliases with one JSON-backed joined query.
It resolves candidate manifests and local results with bulk queries and bounded chunks.
Dirty files still require source reads and hashes because Git identities cannot validate their worktree bytes.
The evaluator publishes those files in batches of at most 64 files.
The evaluator loads tape frames in bulk before worker execution.
The per-file restoration helper receives in-memory manifests only and cannot query SQLite.
Missing frames return to parsing without a point lookup.
Overlay deletion and Git-tree publication also use bulk statements.

The language specification is [Tape transducer language](transducer-design.md).
It defines one-word instructions, tables, tape navigation, tests, captures, alternatives, and output.
It does not specify storage or SQL integration.

## Workload evidence

### Reference corpus

A full measurement over a large Python repository contained:

- 5,725 files.
- 95,697,853 source bytes.
- 160.38 MB of raw compact tapes.
- 56,079,347 bytes of level-3 Zstandard frames.
- 169,014 violations.

The source and tape ratio was approximately 2.68 raw bytes per source byte.
Level-3 Zstandard reduced the combined source and tapes to 21.9 percent of their raw size.

Independent frames gave file-access p50 latency between 59 and 77 microseconds.
File-access p95 latency was between 374 and 497 microseconds.

The measured compression curve supports level 3 as the default:

| Zstandard level | Stored bytes | Compression time |
| --------------: | -----------: | ---------------: |
|               1 |   58,263,964 |           466 ms |
|               3 |   56,079,347 |           590 ms |
|               5 |   53,583,594 |         1,269 ms |
|               7 |   51,590,489 |         2,113 ms |

Level 1 is the low-CPU option.
Level 5 is the size-biased option.
Levels 7 and higher do not save enough space for their compression cost.

### Active-machine inventory

A machine inventory on 2026-09-24 found:

- 45 active Pi processes.
- 44 unique workspaces.
- 12.19 GiB of Pi-process RSS.
- 327,565 maintained Python and TypeScript files.
- 5.05 GB of path-addressed source.
- 1.47 GB of globally unique source.

The content-addressed inventory used the language identity with each source digest.
This rule prevents reuse across incompatible grammars.

The measured cross-workspace reduction was approximately 71 percent.
More workspaces from the same repositories will usually increase this reduction.

### Temporary-tree risk

The same inventory found 58 GB of source-shaped files under directories named `tmp`.
The current scanner does not exclude these directories.

Thirty-five workspaces contained scanner-visible temporary files.
Ten workspaces exceeded the 512 MiB source limit.
Five workspaces exceeded the 100,000-file limit.

The store must obey repository ignore rules or an explicit Rerail ignore contract.
Disposable test trees must not consume the machine cache.

## Current daemon context

The installed daemon can host several workspace runtimes in one process.
Its runtime key includes the storage root, model path, session database, and workspace root.
Each workspace has separate session state, code-rule state, parse state, and result state.

The installed daemon shares its process, native module, model files, and machine rule catalog.
Linked Git worktrees also share the repository identity from the Git common directory.

Cross-process sharing occurs through SQLite and the operating-system page cache, not JavaScript objects.

## Storage alternatives

### Path-addressed storage

Path-addressed hybrid storage required approximately 3.0 GiB for the 44 maintained workspaces.
The scan-limit upper estimate was approximately 6.8 GB.

One hundred similar workspaces can approach the 10 GB limit.
This model also leaves little space for WAL files, churn, session databases, and temporary operations.

### Content-addressed storage

Global content addressing required approximately 0.87 GiB for the measured payload.
Snapshot manifests and path metadata add approximately 0.1 to 0.4 GB.

One hundred mostly identical workspaces will probably require 2 to 4 GB.
The estimate includes payload, manifests, overlays, and normal WAL headroom.

The content-addressed model is the selected design.

### RAM-only alternatives

The 44-workspace inventory gives these approximate RAM requirements for storage alone:

| Representation            | Path-addressed | Content-addressed |
| ------------------------- | -------------: | ----------------: |
| Compressed frames         | 2.8 to 2.9 GiB |  0.85 to 0.95 GiB |
| Restored source and tapes |   13 to 15 GiB |    3.8 to 4.2 GiB |

Path-addressed restored storage needs approximately 15 to 18 GiB after runtime overhead.
Temporary trees can increase this value to more than 30 GiB within current scan limits.

Content addressing reduces RAM only when processes share mappings or one process owns the object cache.
A digest name alone does not remove duplicate buffers from independent processes.

The selected hybrid design keeps the CAS on disk and retains only bounded hot data.

## Database selection

SQLite is the selected database.
The Rust implementation uses `rusqlite` with bundled SQLite.

The store uses these settings as its initial configuration:

```sql
PRAGMA journal_mode=WAL;
PRAGMA synchronous=NORMAL;
PRAGMA foreign_keys=ON;
PRAGMA busy_timeout=250;
PRAGMA cache_size=-16384;
PRAGMA journal_size_limit=16777216;
```

The implementation can change the cache limit after memory measurements.
The machine-wide SQLite cache must remain bounded.

### SQLite and Turso measurements

The comparison used 5,725 rows and 56,079,347 payload bytes.
Both databases used WAL mode, normal synchronization, and an 8 MiB cache.

| Operation                    |   SQLite | Turso 0.7.2 |
| ---------------------------- | -------: | ----------: |
| One bulk-load transaction    | 265.2 ms |    177.5 ms |
| Sequential scan              |  22.0 ms |     42.9 ms |
| 10,000 random reads          |  76.9 ms |    101.4 ms |
| 500 changed-row transactions |  18.6 ms |     59.2 ms |
| Four-worker random reads     | 100.8 ms |     77.5 ms |
| Bulk-load peak RSS           | 143.9 MB |    227.4 MB |

Turso improved bulk loading and synthetic concurrent reads.
SQLite improved sequential reads, point reads, changed-row writes, and bulk-load memory.

Turso also rejected updates to `WITHOUT ROWID` tables in the tested stable release.
The error was `UPDATE of WITHOUT ROWID tables is not supported`.

The selected workload uses short write transactions and permits bounded retries.
Turso does not give a garbage-collection advantage for this workload.

## Store ownership

The daemon uses two persistence scopes.

### Per-worktree state

Each worktree keeps its existing local state.
This state includes session databases, settings, repository audits, and other checkout-owned data.

The CAS records each workspace identity and current root.
The local state does not contain source frames or tape frames.
Rust owns access to this state when it uses a database.

### Machine-wide CAS

The machine-wide CAS stores:

- compressed source frames.
- compressed tape frames.
- canonical content hashes.
- Git object aliases.
- interned strings and paths.
- Git tree metadata.
- immutable snapshots.
- mutable workspace overlays.
- workspace leases.
- garbage-collection metadata.

The default location is below `RERAIL_HOME`.
The database must reside on a local filesystem.

Multiple daemon processes can open this database.
SQLite serializes their short write transactions.
Each caller retries `SQLITE_BUSY` with bounded jitter.

Each process uses one collection thread for each open store.
Collection starts only when the store crosses a capacity threshold or receives an explicit request.
SQLite serializes concurrent collectors.
A collector skips an optional cycle when another process owns the writer lock.

## Fixed-width relational model

Hot tables use integer primary keys and integer foreign keys.
Strings and wide hashes occur only in auxiliary tables.

SQLite does not promise fixed-width physical row records.
This schema gives Rust a fixed logical layout and gives hot indexes integer-only keys.
Random hash lanes normally use the full eight-byte SQLite integer representation.
Surrogate identifiers remain compact in tables that use small values.

SQLite stores `INTEGER` values as signed 64-bit values.
Rust owns the conversion between digest bytes and signed lanes.
JavaScript never receives these values.

### Hash lanes

SHA-256 uses four 64-bit lanes in network byte order.
Rust decodes each lane with `i64::from_be_bytes`.

SHA-1 uses the first two complete lanes and four bytes in the third lane.
The remaining bytes are zero.
The algorithm identifier distinguishes SHA-1 from SHA-256.

```sql
CREATE TABLE hashes (
    hash_id      INTEGER PRIMARY KEY,
    algorithm_id INTEGER NOT NULL,
    lane0        INTEGER NOT NULL,
    lane1        INTEGER NOT NULL,
    lane2        INTEGER NOT NULL,
    lane3        INTEGER NOT NULL,
    UNIQUE (algorithm_id, lane0, lane1, lane2, lane3)
) STRICT;
```

The lane order and padding rules are part of the database format.
Golden-vector tests must protect these rules.

All other tables reference `hash_id`.
No hot table repeats the four hash lanes.

### Static identifiers

Small closed vocabularies use Rust enums and integer columns.
These vocabularies include:

- hash algorithms.
- codecs.
- grammar identities.
- object kinds.
- workspace states.
- overlay states.
- Git entry modes.

The database does not store names for these values.
Rust diagnostics can map each value to a stable name.

### String interning

The store does not create string-based keys for paths or symbols.
The string table stores each text value once.
Its unique index contains only integer values.

```sql
CREATE TABLE strings (
    string_id         INTEGER PRIMARY KEY,
    domain_id         INTEGER NOT NULL,
    content_hash_id   INTEGER NOT NULL REFERENCES hashes(hash_id),
    collision_ordinal INTEGER NOT NULL DEFAULT 0,
    value             TEXT NOT NULL,
    UNIQUE (domain_id, content_hash_id, collision_ordinal)
) STRICT;
```

The Rust interner performs these actions in one transaction:

1. It hashes the UTF-8 bytes.
2. It finds rows with the same domain and hash.
3. It compares the stored text with the input text.
4. It reuses the matching `string_id`.
5. It allocates a collision ordinal when the text differs.

The comparison preserves correct behavior after a theoretical hash collision.
The text value does not occur in a database index.

Initial string domains include paths, path components, workspace roots, and repository identities.

### Paths

The first implementation interns complete normalized repository-relative paths.
This choice keeps violation rendering and path lookup simple.

```sql
CREATE TABLE paths (
    path_id   INTEGER PRIMARY KEY,
    string_id INTEGER NOT NULL UNIQUE REFERENCES strings(string_id)
) STRICT;
```

The same path receives one `path_id` across snapshots and workspaces.
A component trie is a later optimization.
Measurements must justify its additional joins.

## Content objects

### Source objects

The source table stores each canonical source frame once.

```sql
CREATE TABLE source_objects (
    source_id         INTEGER PRIMARY KEY,
    canonical_hash_id INTEGER NOT NULL UNIQUE REFERENCES hashes(hash_id),
    raw_length        INTEGER NOT NULL,
    codec_id          INTEGER NOT NULL,
    frame             BLOB NOT NULL
) STRICT;
```

Source frames use independent level-3 Zstandard compression.
The raw length and canonical hash remain outside the frame.

### Git aliases

A clean Git blob can identify a source object without another filesystem hash.
The Git object ID becomes an alias for the canonical source object.

```sql
CREATE TABLE source_aliases (
    alias_hash_id INTEGER PRIMARY KEY REFERENCES hashes(hash_id),
    source_id     INTEGER NOT NULL REFERENCES source_objects(source_id)
) STRICT;
```

The alias namespace includes the Git hash algorithm.
Git SHA-1 and Git SHA-256 aliases cannot conflict.

On the first encounter, Rust reads the source and computes the canonical SHA-256.
Rust performs this work while it parses and compresses the same bytes.
Later workspaces use the Git alias without another source read.

### Tape objects

A tape depends on source identity, grammar identity, grammar version, and tape format.

```sql
CREATE TABLE tape_objects (
    tape_id             INTEGER PRIMARY KEY,
    source_id           INTEGER NOT NULL REFERENCES source_objects(source_id),
    canonical_hash_id   INTEGER NOT NULL REFERENCES hashes(hash_id),
    grammar_id          INTEGER NOT NULL,
    grammar_version     INTEGER NOT NULL,
    tape_format_version INTEGER NOT NULL,
    codec_id            INTEGER NOT NULL,
    raw_length          INTEGER NOT NULL,
    frame               BLOB NOT NULL,
    UNIQUE (
        source_id,
        grammar_id,
        grammar_version,
        tape_format_version
    )
) STRICT;
```

Source and tape frames remain independent Zstandard frames.
A caller can fetch one frame without reading the other frame.
The runtime validates, hashes, and serializes each batch before it starts the SQLite transaction.
It looks up canonical identities before compression starts.
It compresses only source and tape objects that are not in the store.
Four symmetric workers share parsing, matching, tape construction, hashing, and compression work.
A publication transaction contains at most 64 files.
A busy concurrent writer causes a complete transaction retry.
The runtime reports the retry count as `workspaceProfile.storageRetries`.

### Documents of a file

A file can be several documents.
Its primary document is its own tree over its bytes.
An embedder can derive more documents from it: Blade's compiled view, a Vue component's script program.

Every tape holds exactly one source, one grammar, and one tree (tape format 7, magic `TMPTAPE7`).
No tape holds another tape, a derived text, or a source map.
A derived document is stored as ordinary objects: its text is a `source_objects` row, deduplicated by SHA-256, and its tree is a `tape_objects` row over that text.
The derived tape has the primary tape's grammar version and format.
Its grammar ID is the derived dialect's tape tag with bit 8 set and, above it, a fingerprint of the placeholders its source map omits (0 where it omits none).
A derived tape is its text's tree less those placeholders, so one text can hold two tapes under maps omitting different ones; the grammar ID keeps each its own row, never a primary's, and derived documents with the same text and omissions share one.

A sparse table relates derived documents to the primary document's tape:

```sql
CREATE TABLE file_documents (
    tape_id          INTEGER NOT NULL REFERENCES tape_objects(tape_id) ON DELETE CASCADE,
    ordinal          INTEGER NOT NULL CHECK(ordinal > 0),
    parent_ordinal   INTEGER NOT NULL CHECK(parent_ordinal >= 0 AND parent_ordinal < ordinal),
    role_id          INTEGER NOT NULL CHECK(role_id > 0),
    via_id           INTEGER NOT NULL,
    document_tape_id INTEGER NOT NULL REFERENCES tape_objects(tape_id),
    source_map       BLOB NOT NULL,
    host_errors      BLOB NOT NULL,
    PRIMARY KEY (tape_id, ordinal)
) STRICT, WITHOUT ROWID;
CREATE INDEX file_documents_by_document ON file_documents(document_tape_id);
```

The key is the primary tape, not a path: a file's documents are a function of its source, grammar version, and tape format.
Ordinal 0 is the primary document and has no row.
A single-language file has no rows, so its stored objects are unchanged.
`parent_ordinal` names the document a row's document was derived from; embedders derive from the primary only.
`role_id` is 1 for an embedded document, whose text is one contiguous byte-exact slice of its parent (one copied segment over the whole text), and 2 for a generated one (any other source map).
`via_id` is the embedding's registry name read as a nonnegative integer: the first eight bytes of its SHA-256.
`source_map` holds the segment map's persisted segments; `host_errors` holds the parent byte ranges the document cannot be read through, as little-endian `u32` pairs.

Publication writes the primary tape, every derived source and tape, and their `file_documents` rows in one object transaction.
Only a tape that is not stored yet writes its documents: a stored tape's documents were written with it.
Loading reads a file's primary tape, its rows, and its derived tapes and texts in bulk, then checks each derived document against its parent.
A missing or invalid derived part makes the stored tape absent: the file is parsed again, never restored without its derived documents.
The search indexer reads a derived tape as an ordinary row, in its own dialect over its own text, so every document has its own search index.
A clause over a derived document consults that document's index.

## Git implementation boundary

Rerail uses `gix` for Git repository and worktree discovery.
The Rust process reads objects, references, indexes, attributes, and worktree state through `gix`.
Production discovery code does not execute an external `git` binary.
Thus, discovery does not depend on a Git installation or subprocess output.

SQLite remains the owner of persisted Rerail metadata.
The `gix` object model does not cross the Rust boundary into JavaScript.

## Git tree reuse

Git tree IDs form a Merkle graph over names, modes, blobs, and child trees.
An unchanged subtree retains the same tree ID across commits and worktrees.

Rerail uses Git tree IDs to:

- identify immutable base snapshots.
- skip unchanged directory walks.
- reuse expanded path manifests.
- find known Git blob aliases.
- attach a workspace to an existing snapshot.

Git tree IDs do not replace canonical leaf identities.
They provide snapshot and discovery identities.

### Git tree tables

```sql
CREATE TABLE git_trees (
    tree_id      INTEGER PRIMARY KEY,
    tree_hash_id INTEGER NOT NULL UNIQUE REFERENCES hashes(hash_id),
    complete     INTEGER NOT NULL DEFAULT 0 CHECK(complete IN (0,1))
) STRICT;

CREATE TABLE git_tree_entries (
    tree_id        INTEGER NOT NULL REFERENCES git_trees(tree_id),
    name_string_id INTEGER NOT NULL REFERENCES strings(string_id),
    mode_id        INTEGER NOT NULL,
    object_kind_id INTEGER NOT NULL,
    target_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    PRIMARY KEY (tree_id, name_string_id)
) STRICT, WITHOUT ROWID;
```

A tree row with `complete=1` has its full entry set.
Publication writes a tree's entries and sets the marker in one transaction, and it skips only complete trees.
Rows written before the marker existed stay unproven until publication re-sends their entries.
A snapshot requires a complete root tree; the store never creates an empty root row on its behalf.

The first version can omit full tree expansion from SQLite.
It can cache only expansions that reduce measured indexing work.

### Workspace ignore rules

Git ignore rules control the untracked paths that `gix` returns.
Rerail applies `.rerailignore` after Git ignore rules.
The `.rerailignore` file uses Git ignore syntax and supports negated patterns.

Rerail always excludes these directory names:

- `.git`
- `node_modules`
- `target`
- `tmp`
- `__pycache__`

A `.rerailignore` negation cannot include a path from this mandatory list.
`gix` receives these exclusions as pathspecs before its worktree walk.
This rule prevents disposable trees from entering discovery or capacity measurements.

These rules shape Git discovery, and so storage reuse, only. A repository audit evaluates every file
the host's walk selects, including ignored and mandatory-excluded paths; it reads them from the
worktree instead of restoring them.

### Git limitations

A Git tree describes committed bytes.
The evaluated workspace can contain different bytes.

The implementation represents a workspace as an immutable base snapshot plus a mutable overlay.
The overlay includes staged, unstaged, untracked, and deleted paths.

Content-transforming Git attributes need special handling.
These attributes include filters, line-ending conversion, and `working-tree-encoding`.

If a transform can change workspace bytes, Rust reads and hashes the workspace file.
Rerail violations and tape spans must bind to the evaluated workspace bytes.

A Git submodule entry identifies a commit, not a child tree.
Rerail treats each scanned nested repository as a separate snapshot root.
The scanner continues to ignore symbolic links.

## Snapshots and overlays

A snapshot is immutable.
Its identity includes every version that changes discovery or tape meaning.

```sql
CREATE TABLE snapshots (
    snapshot_id         INTEGER PRIMARY KEY,
    root_tree_id        INTEGER NOT NULL REFERENCES git_trees(tree_id),
    scanner_version     INTEGER NOT NULL,
    grammar_set_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    tape_format_version INTEGER NOT NULL,
    UNIQUE (
        root_tree_id,
        scanner_version,
        grammar_set_hash_id,
        tape_format_version
    )
) STRICT;

CREATE TABLE snapshot_files (
    snapshot_id INTEGER NOT NULL REFERENCES snapshots(snapshot_id),
    path_id     INTEGER NOT NULL REFERENCES paths(path_id),
    source_id   INTEGER NOT NULL REFERENCES source_objects(source_id),
    tape_id     INTEGER REFERENCES tape_objects(tape_id),
    PRIMARY KEY (snapshot_id, path_id)
) STRICT, WITHOUT ROWID;
```

One hundred workspaces on the same commit reference one snapshot.
The snapshot contains one path map and one set of content references.

The overlay records differences from the base snapshot.

```sql
CREATE TABLE workspaces (
    workspace_id    INTEGER PRIMARY KEY,
    root_path_id    INTEGER NOT NULL REFERENCES paths(path_id),
    repository_id   INTEGER NOT NULL,
    snapshot_id     INTEGER NOT NULL REFERENCES snapshots(snapshot_id),
    generation      INTEGER NOT NULL,
    lease_until     INTEGER NOT NULL,
    state_id        INTEGER NOT NULL
) STRICT;

CREATE TABLE workspace_overlay (
    workspace_id INTEGER NOT NULL REFERENCES workspaces(workspace_id),
    path_id      INTEGER NOT NULL REFERENCES paths(path_id),
    state_id     INTEGER NOT NULL,
    source_id    INTEGER REFERENCES source_objects(source_id),
    tape_id      INTEGER REFERENCES tape_objects(tape_id),
    PRIMARY KEY (workspace_id, path_id)
) STRICT, WITHOUT ROWID;
```

A deleted overlay path has no source or tape reference.
An added or replaced path references the new immutable objects.

## Database operations

Rust exposes domain operations to the daemon.
The initial operation set includes:

- open or renew a workspace.
- attach a Git snapshot.
- apply a bounded overlay batch.
- fetch one source frame.
- fetch one tape frame.
- resolve a path to immutable objects.
- retire a workspace.
- read bounded store statistics.
- request maintenance.

These operations do not expose SQL concepts.
They do not expose database row identifiers outside Rust.

Rust creates and compresses frames before it starts a write transaction.
The transaction only interns identifiers, inserts immutable rows, and publishes references.

Duplicate frame creation is safe.
Competing writers use insert-or-find behavior and discard the duplicate frame after commit.

A normal edit transaction changes one overlay path and its object references.
An indexing transaction publishes a bounded batch.
No transaction contains a complete repository audit.

## Garbage collection

The database relationships are the liveness authority.
The implementation does not use an application-maintained reference count as the authority.

Garbage collection runs in the Rust daemon as bounded background work.
The schedule changes collection time, but it does not change liveness.

### Live roots

These records keep objects live:

- active workspace roots.
- retained immutable snapshots.
- workspace overlays.
- `file_documents` rows, for the derived tapes they name and so those tapes' sources.
- explicit pins from indexing workers.
- explicit pins from in-flight edits.

Workspace leases include an owner token and an expiration time.
A missing Pi process does not cause immediate deletion.
Explicit worktree retirement starts collection without waiting for lease expiration.

### Collection order

The collector uses this order:

1. It marks expired workspaces as retiring.
2. It removes overlay rows in bounded batches.
3. It removes unreferenced snapshots in bounded batches.
4. It removes unreferenced tape objects in bounded batches. Removing a primary tape cascades to its `file_documents` rows; a later batch then removes the derived tapes no row names.
5. It removes unreferenced source objects in bounded batches.
6. It removes unused paths, strings, hashes, and Git tree metadata. It deletes an unreachable Git tree whole: the tree row and all of its entries in the same transaction.
7. It performs bounded WAL and free-page maintenance.

Each delete statement processes at most one collection batch (256 rows, or 256 whole Git trees); larger batches are rejected.
Each transaction uses `BEGIN IMMEDIATE` and finishes in a few milliseconds.

Foreign keys use restrictive deletion for shared objects.
The collector deletes relationship rows before it deletes shared objects.
Large cascading deletes are not part of the normal path.
Every reference probe in a collection statement uses an index; no correlated subquery scans a table per candidate row.

### Schema compatibility

SQLite `user_version` is the compatibility floor.
A binary refuses a store whose floor is newer than it supports, so the floor moves only when an older binary would corrupt or misread the store.
Floor 17 dropped the exact-value and source-word FTS families and introduced the Git tree completeness marker.
Additive changes that older SQL ignores, such as indexes, raise the store revision instead.
Revision batches are idempotent, and an older binary with the same floor accepts a newer revision.

### Collection races

The collector rechecks liveness in each delete statement.
A stale candidate list cannot delete an object that received a new reference.

Publication does not synchronize with collection.
If collection removes an unreferenced object, the next publication read finds the missing object.
The publication loop then inserts the object again.
The completion marker remains absent until reconciliation finds all required rows.

New objects receive a creation generation.
A collection cycle does not delete objects from its own generation.
This rule reduces unnecessary correction work.

### SQLite file maintenance

Deleted rows create reusable free pages.
SQLite reuses these pages without shrinking the database file.

The store enables incremental vacuum when it creates the database.
The daemon runs small incremental-vacuum steps after a free-page threshold.

The daemon does not run frequent full `VACUUM` operations.
A full vacuum needs long write access and temporary disk space.

## Resource control

### Disk limits

The store uses these initial machine thresholds:

- 7 GB soft limit.
- 9 GB hard limit.
- 10 GB absolute budget.

The soft limit starts background collection.
The hard limit blocks new cache retention after the current transaction.
Required edit evaluation can continue with transient frames or direct recomputation.

The database records payload bytes, WAL bytes, free pages, and object counts.
Rust reports these values through bounded daemon statistics.

### Memory limits

The CAS remains on disk.
The daemon does not retain all compressed frames in RAM.

The memory plan uses bounded regions for:

- SQLite page caches.
- hot compressed frames.
- restored source and tapes.
- active code-rule runtimes.
- garbage-collection scratch state.
- result delivery.

Inactive workspace runtimes must leave memory.
A workspace manifest in SQLite does not require a resident code-rule runtime.

The daemon must share immutable compiled code-rule plans when their identities match.
It must retain only active workspace overlays and hot file frames.

Initial parsing and indexing remain in a disposable worker.
Process exit releases parser and allocator high-water pages.

All workspaces in one process share one `ContentStore` for each database path.
They share the decompressed-object cache and the maintenance infrastructure.
Each worker thread owns one SQLite connection for each database path that it uses.
WAL mode permits reads on these connections while another thread or process writes.
SQLite serializes the short `BEGIN IMMEDIATE` publication passes.
One maintenance thread serves all workspaces that use the same store in one process.
The thread renews all local workspace leases with one bulk statement.
It does not run collection while the store remains below the soft limit.

The decompressed-object cache has a 32 MiB logical limit and a 30-second sliding lifetime.
It stores decompressed sources and tapes in one 64 MiB anonymous mapping.
The database stores both object types as independent Zstandard frames.
Garbage collection copies live sources and tapes into one replacement mapping.
The old mapping remains readable until active readers release it.
Thus, the cache has no more than two mappings during collection.
The store applies `MADV_DONTNEED` before it removes a mapping.
An oversized source or tape uses bounded transient heap storage instead of a separate mapping.

## Failure and recovery

The CAS contains rebuildable data.
A database failure must not corrupt source worktrees or session databases.

SQLite transactions protect all published references.
An interrupted frame build can leave only an unreferenced immutable object.
A later collection cycle removes that object.

The daemon validates these fields before frame use:

- codec identity.
- raw length.
- canonical source or tape hash.
- grammar identity.
- grammar version.
- tape format version.

A failed validation removes the cache entry from use.
Rerail rebuilds the object from source.

The daemon must reject a database with a newer schema version.
Migrations run in Rust before normal operations start.

## Validation plan

The implementation must provide these tests and measurements:

1. Hash-lane golden vectors for SHA-1 and SHA-256.
2. String-interner collision tests.
3. Git alias tests across repeated worktrees.
4. Tree snapshot reuse across branch changes.
5. Overlay tests for staged, unstaged, untracked, and deleted files.
6. Attribute tests for line endings, filters, and workspace encodings.
7. Tape parity tests with exact roots and complete capture bindings.
8. Crash tests at every object-publication boundary.
9. Concurrent writer and garbage-collector tests.
10. Lease expiry and explicit retirement tests.
11. Database corruption and stale-version tests.
12. A 100-workspace scale test with a pinned corpus.

The final scale test must meet these limits:

- steady-state Rerail RSS below 1 GiB.
- store size below 10 GB.
- initial worker peak RSS below 500 MiB.
- edit p95 below 500 ms.
- no lost live object after concurrent collection.
- no retained object after retirement and a complete grace period.

A repository check must reject JavaScript SQLite dependencies and SQL strings in Rerail JavaScript files.
Only Rust storage modules can open the CAS database.

## Decisions

The design makes these decisions:

- Use a machine-wide content-addressed store.
- Use SQLite through Rust only.
- Keep JavaScript outside the database boundary.
- Store SHA values as four signed 64-bit lanes.
- Reference hashes through integer surrogate keys.
- Intern strings through integer IDs and fixed-width hash indexes.
- Use integer IDs in all hot tables and relationships.
- Compress source and tape frames independently with Zstandard level 3.
- Use Git blob IDs as aliases.
- Use Git tree IDs for snapshot and subtree reuse.
- Represent dirty worktrees as immutable snapshots plus overlays.
- Use database relationships as the liveness authority.
- Run bounded garbage collection in the Rust daemon.
- Keep inactive workspace runtimes out of memory.

## Deferred decisions

The first implementation does not require these features:

- a normalized component trie for all paths.
- a fully normalized Git tree DAG as the ownership graph.
- application-maintained reference counters.
- Turso Cloud synchronization.
- cross-machine cache replication.
- a public database or SQL interface.
- JavaScript access to SQLite.

Measurements can justify a component trie or a normalized tree DAG later.
These changes must preserve the Rust-only database boundary.
