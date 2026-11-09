# Content-addressed workspace store

The content store is a SQLite cache of parsed source shared by every workspace that opens the same
file. With it, a repository index restores clean files instead of reading and evaluating them again.
Losing it costs reuse, never results.

<details>
<summary>What it holds and where the code lives</summary>

It holds compressed sources and syntax tapes, per-file rule results, Git tree and snapshot metadata,
workspace overlays of changed files, and a kind search index.

Code: `native/src/content_store/`, plus `native/src/code_rule_dsl/workspace/` (`storage.rs`,
`persisted.rs`, `index.rs`, `documents.rs`). File location and owning process:
[How setup works](how-setup-works.md#the-content-store).

</details>

## Boundary

Rust owns the database; JavaScript sees only a path and a few coarse operations.

- A workspace attaches to the store only inside a Git worktree; elsewhere it runs without one.
- Files named like secret holders are never written.

<details>
<summary>The JavaScript calls and withheld files</summary>

JavaScript passes the store's path to `enableStorageAsync()` and calls `applyBufferAsync()`,
`indexRepositoryAsync()`, `completeStorageRevisionAsync()` and `resultPageAsync()`
(`src/engine/native/native.ts`). Connections, statements, transactions and schema changes stay in
Rust; no SQL, row identifier, hash lane or database error reaches JavaScript.

The secret-holder names are the ones `enableStorageAsync` receives. Such a file gets no stored
source, tapes or results (`StorageContext::withholds`).

</details>

## Schema and compatibility

One idempotent DDL creates the schema; a version floor keeps older builds off newer stores.

The floor rises only when an older binary would corrupt or misread the store. Raising it renames
the file, so the new store starts empty.

<details>
<summary>How opening and the floor work</summary>

`schema.rs` holds the DDL. Opening a complete store only reads, so it never waits on a writer. A
store missing any object (new, or lacking an index this build added) takes the exclusive lock and
applies the DDL; a new store also gets `auto_vacuum=INCREMENTAL`.

`PRAGMA user_version` is the compatibility floor, `SCHEMA_VERSION` (1). A build refuses a store
whose floor is newer than its own. Additive changes such as indexes never raise it.

The renamed file is `STORE_FILE_NAME`, now `content-v1.sqlite`. The old file is neither migrated nor
deleted.

Tests: `newer_compatibility_floor_is_refused`,
`reopening_restores_missing_indexes_without_raising_the_floor`,
`opening_a_current_store_never_needs_the_write_lock`.

</details>

## Connections and busy writers

Each thread reads on its own connection; a busy writer retries, then leaves the store alone.

A write that stays busy for 10 s gives up. The workspace then skips the store for 30 s and keeps its
results in memory.

<details>
<summary>Connection settings</summary>

`ContentStore` holds `connection: ThreadConnection`, one connection per thread and database path
from the thread-local `THREAD_CONNECTIONS`. Rayon workers, libuv pool threads and the maintenance
thread thus read in parallel under WAL. `release_memory()` closes idle connections; the next read
reopens them.

`configure_connection` sets each connection to:

```sql
PRAGMA foreign_keys=ON;
PRAGMA journal_mode=WAL;
PRAGMA synchronous=NORMAL;
PRAGMA cache_size=-1024;          -- 1 MiB page cache per connection
PRAGMA mmap_size=0;
PRAGMA journal_size_limit=16777216;
```

It also sets a 128-entry statement cache and a busy handler, `wait_for_writer`, waiting up to
`BUSY_TIMEOUT` (250 ms) per attempt for the write lock.

</details>

<details>
<summary>Retry, give-up and cooldown details</summary>

1. **Retry.** Every write runs under `retry_busy`: a `Busy` attempt rolls back and re-runs whole
   after jittered backoff, doubling from 2 ms to 200 ms.
2. **Give up.** After `BUSY_DEADLINE` (10 s) the write returns `StoreError::Busy`. An
   `OperationDeadline` shares one deadline across an operation's writes (an apply's publication
   passes, a snapshot's batches).
3. **Cool down.** The workspace skips the store for `STORE_BUSY_COOLDOWN` (30 s), keeping results in
   memory; the first apply afterwards publishes them.

A workspace result's `workspaceProfile` reports `storageRetries` and `storageRetryWaitNanoseconds`.

</details>

## Hashes, strings and paths

Hashes, text and paths are each stored once and referenced by integer ID.

- **`hashes`:** each hash identity once; other tables reference its `hash_id`.
- **`strings`:** text interned per domain.
- **`paths`:** whole workspace-relative paths, one `path_id` across snapshots and workspaces.

<details>
<summary>Hash encoding and lookup</summary>

A `HashKey` is an algorithm (`HASH_SHA256` 1, `HASH_GIT_SHA1` 2, `HASH_GIT_SHA256` 3) and four
signed 64-bit lanes, each eight digest bytes read with `i64::from_be_bytes`. SHA-1 is zero-padded to
32 bytes (test: `hash_lanes_are_stable_and_sha1_is_padded`).

- **`hashes(hash_id, algorithm_id, lane0..lane3)`:** its only index is
  `hashes_by_prefix ON hashes(algorithm_id, lane0)`; lookups compare the other three lanes. With no
  unique index on the full identity, writers insert missing hashes inside `BEGIN IMMEDIATE`, which
  serializes them (test: `hash_lookup_index_contains_only_algorithm_and_first_lane`).
- **`strings(domain_id, content_hash_id, collision_ordinal, value)`:** the domains are path,
  repository, owner, path component and clause key. Lookup is by SHA-256, then value; differing text
  with the same hash gets the next `collision_ordinal`. Text is never indexed.

</details>

## Objects

Each source and tape is stored once, as an independent compressed frame checked on every read.

- **`source_objects`:** each source under its SHA-256.
- **`tape_objects`:** one syntax tape per source and grammar.
- **`source_aliases`:** a Git blob ID mapped to its source.

<details>
<summary>Columns, aliases and integrity checks</summary>

- **`source_objects`** stores raw length, codec (1, Zstandard) and frame.
- **`tape_objects`** stores one tape per source, `grammar_id`, `grammar_version` (the grammar build
  identity) and `tape_format_version`.

Each frame is an independent level-3 Zstandard frame, readable alone. Both tables carry a
`created_generation` for collection.

Publication adds an alias when evaluated bytes, from a repository index or host edit, hash to the
committed blob. Only files whose path is clean in the worktree are hashed (`clean_blob_alias`). An
alias naming different bytes fails publication as corrupt.

Reads check codec, raw length and SHA-256 against the row, failing with `StoreError::Corrupt` (test:
`corrupt_zstandard_frames_fail_closed`). A stored tape that does not load is parsed again.

</details>

### Documents of a file

A file can be several documents: its primary tree over its own bytes, plus any an embedder derives,
such as Blade's compiled view or a Vue component's script program.

Derived documents are ordinary source and tape objects. `file_documents` links them to the primary
tape.

<details>
<summary>How derived documents are stored</summary>

Every tape holds one source, one grammar and one tree (tape format 7, magic `TMPTAPE7`). A derived
document is ordinary objects:

- **Text:** a `source_objects` row.
- **Tree:** a `tape_objects` row over that text, with the primary tape's grammar version and format.
- **Grammar ID:** the dialect's tape tag with bit 8 set, topped by a fingerprint of the placeholders
  its source map omits (0 for none).

So one text can hold several derived tapes, never a primary's; derived documents with the same text
and omissions share one (`document_grammar_id`).

`file_documents` relates derived documents to the primary tape:

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
```

Keyed by primary tape, not path, a file's documents follow from its source, grammar version and tape
format. Ordinal 0, the primary, has no row, so a single-language file has none.

- **`parent_ordinal`:** the document this one derives from.
- **`role_id`:** 1 embedded (a byte-exact slice of its parent), 2 generated.
- **`via_id`:** first eight bytes of the SHA-256 of the embedding's registry name, as a nonnegative
  integer (`via_code`).
- **`source_map`:** the persisted segments.
- **`host_errors`:** parent byte ranges the document cannot be read through, as little-endian `u32`
  pairs.

</details>

<details>
<summary>Publication and loading</summary>

Publication writes a new primary tape, its derived sources and tapes, and their rows in one
transaction; a tape already stored keeps its rows. Loading reads a primary tape, its rows and its
derived objects in bulk (`file_documents_bulk`). If any part is missing or fails its checks, the file
is parsed again rather than restored without its derived documents.

Each derived tape gets its own search index, which clauses over that document consult.

</details>

## Git snapshots and overlays

A snapshot records what a commit holds; a workspace's overlay records what it evaluated on top.

- **Snapshot** (`snapshots`, `snapshot_files`): a commit's paths mapped to sources and tapes, shared
  by workspaces on that commit.
- **Overlay** (`workspace_overlay`): a workspace's paths mapped to the sources and tapes it evaluated.
- **Discovery** (`native/src/git_workspace.rs`) reads repositories, indexes, attributes and worktree
  state with `gix`; production code runs no `git` binary.

<details>
<summary>Ignore rules</summary>

Discovery skips:

- built-in ignores (`BUILTIN_IGNORE`: `.git/`, `node_modules/`, `target/`, `tmp/`, `__pycache__/`);
- `.rerailignore` paths, in Git ignore syntax with negation. A negation cannot restore a built-in
  ignore.

These rules limit only what the store may restore, never what a repository index evaluates. A
gitlink (submodule) is recorded as a nested path, not descended into.

</details>

<details>
<summary>Tables and clean-file checks</summary>

- **`git_trees`, `git_tree_entries`:** each Git tree's entries under interned names. A tree is
  complete when its row has `complete=1` and still has the `entry_count` publication wrote, so
  partial entry sets are republished. Missing trees are packed whole into transactions of up to
  `GIT_TREE_BATCH_ENTRIES` (512) entries; a larger tree is written alone.
- **`snapshots`:** unique on root tree, scanner version, grammar set and tape format, so workspaces
  on one commit share a snapshot. `publish_snapshot` registers the workspace in the snapshot's own
  transaction and requires a published root tree (test: `snapshots_require_a_published_root_tree`).
- **`workspaces`:** root, repository (Git common directory), owner (`rerail:<pid>:<worktree>`),
  snapshot, lease and state (1 active, 2 retiring).

A clean file's committed blob is its alias, found without reading the file (`clean_source_alias`).
Before a restore, `holds_blob_bytes` confirms the worktree still holds the blob's bytes: one `lstat`
while the stat matches the index entry, else a hash. The conversions `filter`, `ident` and
`working-tree-encoding` always force the hash, as they can change bytes without changing size.

Publication promotes clean overlay files into the snapshot (`promote_overlay_files_to_snapshot`, at
most 64 paths).

</details>

## Results and reuse

Rule results are cached per file content and context; completion records mark which clean trees may
reuse them.

- **Context results** (`context_check_results`): one file's results for one clause, keyed by source
  and context.
- **Completion record:** `completeStorageRevisionAsync()` marks the clean trees a finished index
  covered.
- **Restore:** a repository index restores a clean file only with results for every clause and its
  blob's bytes still present. It reads and evaluates the rest.

Scan budgets: [Reference](reference.md#scan-and-settings-controls). Result pages are not stored;
`resultPageAsync()` pages an in-memory `ResultPageIndex`.

<details>
<summary>Catalog and result tables</summary>

- **`candidate_catalogs`:** a rule catalog's identity, a SHA-256 over the code-rule catalog, grammar
  set, scanner version and tape format (`CandidateCatalogIdentity`).
- **`candidate_checks`:** one clause of a catalog, by rule (policy hash) and clause key.
- **`context_check_results(source_id, context_hash_id, check_id, codec_id, raw_length, frame)`:**
  per-file results. The context (`file_context`) is all non-content inputs: path, language, Python
  module, nearest `package.json` and any calls-closure stamp (`result_context`).
- **`local_check_results(source_id, check_id, ...)`:** still in the schema and still collected, but
  never written.

`LocalResultCache` keeps up to 32 MiB of decoded results per process.

</details>

<details>
<summary>Completion records</summary>

In a Git worktree, `completeStorageRevisionAsync()` writes
`completed_directory_reuse(tree_hash_id, prefix, package_hash_id, catalog_hash_id)` per clean tree,
plus the workspace's `completed_workspace_revisions` row (`complete_directories`).

The package hash covers what the tree's files read outside it: ancestors' Python package markers and
the inherited `package.json` of its ECMAScript files. If the write fails, the audit keeps its
results and loses only reuse.

</details>

<details>
<summary>How a repository index restores files</summary>

A repository index (`index.rs`):

1. Walks the host-selected files and finds clean trees with a completion record for the current
   catalog (`reusable_directory_prefixes`).
2. Under those prefixes, bulk-resolves clean files' aliases (`stored_sources_by_alias`) and results
   (`local_check_results_by_key`).
3. Restores a file only with results for every clause and its blob's bytes still present. Listing
   and restoring evaluate nothing.
4. Reads the rest in chunks of at most `INDEX_BATCH_FILES` (32) files, closing a chunk at
   `INDEX_BATCH_BYTES` (1 MiB). Each chunk is published, and its sources and tapes released, before
   it returns.

</details>

## Kind search index

The search index tells, without decoding a tape, whether a node kind occurs in it.

When every AST clause's root kinds are absent from a tape, restoring the file skips decoding it.

<details>
<summary>Schema, lookup answers and index building</summary>

```sql
CREATE TABLE search_index_sets (
    tape_id       INTEGER PRIMARY KEY REFERENCES tape_objects(tape_id) ON DELETE CASCADE,
    index_version INTEGER NOT NULL CHECK(index_version >= 0),
    node_count    INTEGER NOT NULL CHECK(node_count > 0),
    complete      INTEGER NOT NULL CHECK(complete IN (0,1)),
    has_errors    INTEGER NOT NULL DEFAULT 0 CHECK(has_errors IN (0,1))
) STRICT;
CREATE VIRTUAL TABLE search_kind_fts USING fts5(
    token, content='', contentless_delete=1, detail=full, tokenize='ascii'
);
```

`search_kind_fts` is contentless FTS5: one document per tape (`rowid` is the tape ID), one kind
token per preorder node, independent of the rule catalog. The trigger `delete_tape_search_index`
drops a tape's document with the tape.

`get_search_kind_presence_bulk` answers `Present`, `Absent`, `SyntaxErrors` or `Unknown`. Only a
complete, error-free set at the requested index version answers `Absent`; missing, stale or
incomplete sets give `Unknown`. When every AST clause's root kinds are `Absent`,
`restore_persisted_file` skips decoding the tape and records no witnesses.

The maintenance thread builds indexes after publication, never during it. It reads unindexed tapes
via a keyset cursor (`tapes_without_search_index_after`), 64 at a time, for about one second per
wake, writing each batch in one transaction (`put_search_indexes_bulk`). A tape this build cannot
decode gets an incomplete set, leaving the backlog.

</details>

## Overlay publication

`publish_overlay_files` publishes 1 to `MAX_PUBLICATION_BATCH` (64) files of one workspace, doing
all it can outside the write lock.

Hashing, lookups and compression run without the lock. An object pass and a relationship pass then
write under it.

Publication alone does not make results reusable: a repository index restores only under a
completion record (above), written after the whole index is published.

<details>
<summary>Work outside the lock, the two write passes, and tests</summary>

Without a write lock it:

1. Validates each file and hashes its source and tape on the worker pool.
2. Looks up stored sources, tapes, paths, catalogs and checks in one deferred read transaction
   (three bulk statements).
3. Compresses only the sources and tapes the store lacks.

**Object pass.** Anything missing is written under `BEGIN IMMEDIATE`. Each statement takes the whole
batch (identities as a JSON parameter, frames as bound BLOBs) and joins by full identity, never by an
ID read earlier. Order: hashes, strings, paths, catalogs, checks, sources, aliases, tapes, derived
documents.

A pass needing over ten data statements fails. A fresh read then checks every row exists, since a
collection between read and write can delete rows the pass relied on; the next pass rewrites them.
Publication fails after eight incomplete passes.

**Relationship pass.** It writes overlay rows, new result-context hashes and `context_check_results`,
chunked to SQLite's variable limit, in at most ten statements. Under the write lock it first checks
every prepared path, source, tape, catalog and check ID is still current
(`relationship_references_current`); otherwise the whole publication restarts, at most eight times.

Every pass runs under `retry_busy`. Cancellation is checked between passes and before commit: an
uncommitted pass rolls back; committed passes stay valid. `PublicationResult` reports statement,
pass, retry and timing counts.

Tests:

- ten-statement budget: `overlay_publication_replaces_payload_and_candidates_atomically`;
- no hashing under the write lock:
  `relationship_revalidation_does_not_hash_payloads_under_write_lock`;
- reconciliation: `reconciliation_reprepares_source_collected_after_the_read`,
  `reconciliation_revalidates_path_collected_before_relationship_write`.

</details>

## Maintenance and collection

A maintenance thread per store renews leases, builds search indexes and collects unreferenced rows
when the store grows.

Each process keeps one `ContentStore` and maintenance thread (`rerail-store-maintenance`) per store
path (`shared_content_store`). Every 30 seconds the thread:

- renews its workspaces' leases to 120 seconds ahead, in one statement;
- runs one collection pass if the capacity state is not `Normal`;
- builds search indexes.

Collection starts at 7 GiB; from 9 GiB new sources, tapes and search indexes are refused. A closed
workspace stops being renewed, so its lease expires and collection retires it.

<details>
<summary>Capacity thresholds</summary>

`capacity()` sums the database, `-wal` and `-shm` files (test: `capacity_thresholds_are_exact`):

| Threshold              | Size   | State          | Effect                                      |
| ---------------------- | ------ | -------------- | ------------------------------------------- |
| `STORE_SOFT_LIMIT`     | 7 GiB  | `Collect`      | collection starts; nothing collected below  |
| `STORE_HARD_LIMIT`     | 9 GiB  | `RejectWrites` | new sources, tapes, search indexes refused  |
| `STORE_ABSOLUTE_LIMIT` | 10 GiB | `OverBudget`   |                                             |

</details>

<details>
<summary>Collection order and safeguards</summary>

Database relationships are the only liveness authority; there are no reference counts.
`collect(now, batch)` takes at most `MAX_COLLECTION_BATCH` (256) rows per statement, in one
`BEGIN IMMEDIATE` transaction, in this order:

1. Expired `object_pins`.
2. Workspaces whose lease expired become retiring; their overlay rows go, then the workspace and its
   `completed_workspace_revisions` row.
3. The files of one snapshot no workspace references, then the snapshot once empty.
4. Tapes no snapshot file, overlay, pin or `file_documents.document_tape_id` references. A deleted
   primary tape takes its `file_documents` rows, search index set and FTS document with it, so a
   later pass deletes the derived tapes no row names.
5. Unreferenced sources, after their aliases and stored results.
6. Checks without results, catalogs without checks, and `completed_directory_reuse` rows whose
   catalog is gone.
7. Git trees no snapshot reaches, each deleted whole with its entries, within the
   `GIT_TREE_BATCH_ENTRIES` budget (always at least one tree).
8. Unreferenced paths, strings and hashes.

Collection deletes only objects with `created_generation` older than the store's generation, then
increments it, so objects published since the last pass survive. After commit it runs
`PRAGMA wal_checkpoint(PASSIVE)` and `PRAGMA incremental_vacuum(128)` and clears process caches.

Every reference probe uses an index; `collect_statements_never_scan_inside_correlated_subqueries`
checks each statement's query plan.

`retire_workspace` and the pin API (`pin_source`, `pin_tape`, `release_pins`) have no production
caller.

</details>

## Memory

Each process bounds its caches and connections; `release_memory()` frees them.

- **Read cache:** up to 32 MiB of decompressed sources and tapes, 30-second sliding lifetime.
- **`LocalResultCache`:** up to 32 MiB of decoded results.
- **Connections:** 1 MiB SQLite page cache each.
- **`release_memory()`** clears both caches and closes idle connections.

<details>
<summary>How the read cache maps memory</summary>

The read cache limits are `DEFAULT_READ_CACHE_BYTES` and `DEFAULT_DECOMPRESSED_CACHE_TTL`. Entries
live in one anonymous mapping twice the size limit.

When full, live entries move to a replacement mapping; the old one stays readable until its readers
drop it. At most two exist, each given `MADV_DONTNEED` before `munmap`. An entry over the limit, or
arriving while a retired mapping is in use, goes on the heap.

</details>
