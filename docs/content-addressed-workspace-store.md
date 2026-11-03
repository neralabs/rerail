# Content-addressed workspace store

The content store is a SQLite cache of parsed source shared by every workspace that opens the same
file. It holds compressed sources and syntax tapes, per-file rule results, Git tree and snapshot
metadata, each workspace's overlay of changed files, and a kind search index over stored tapes. It
lets a repository index restore clean files instead of reading and evaluating them again.

The store is a cache: losing it costs reuse, never results. The code is in
`native/src/content_store/`; the workspace side is in `native/src/code_rule_dsl/workspace/`
(`storage.rs`, `persisted.rs`, `index.rs`, `documents.rs`). Where the file lives and which process
opens it are in [How setup works](how-setup-works.md#the-content-store).

## Boundary

Rust owns every connection, statement, transaction and schema change. JavaScript passes only the
store's path to `enableStorageAsync()` and calls coarse workspace operations: `applyBufferAsync()`,
`indexRepositoryAsync()`, `completeStorageRevisionAsync()` and `resultPageAsync()`
(`src/engine/native/native.ts`). No SQL, row identifier, hash lane or database error crosses to
JavaScript.

A workspace attaches to the store only inside a Git worktree; elsewhere it runs without one. A file
named like one holding secrets (the names `enableStorageAsync` receives) is never written: not its
source, its tapes or its results (`StorageContext::withholds`).

## Schema and compatibility

`schema.rs` holds one idempotent DDL. Opening a store that already has every object it creates only
reads, so it never waits on another writer. A store missing any object (a new store, or an index
this build added) takes the exclusive lock and applies the DDL; a new store also gets
`auto_vacuum=INCREMENTAL`.

`PRAGMA user_version` is the compatibility floor, `SCHEMA_VERSION` (1). A build refuses a store
whose floor is newer than its own. The floor rises only when an older binary would corrupt or
misread the store; additive changes such as indexes do not raise it. Raising it renames the store
file (`STORE_FILE_NAME`, now `content-v1.sqlite`), so a new store starts empty and the old file is
neither migrated nor deleted. Tests: `newer_compatibility_floor_is_refused`,
`reopening_restores_missing_indexes_without_raising_the_floor`,
`opening_a_current_store_never_needs_the_write_lock`.

## Connections and busy writers

`ContentStore` holds `connection: ThreadConnection`, which opens one connection per thread and
database path from the thread-local `THREAD_CONNECTIONS`. Rayon workers, libuv pool threads and the
maintenance thread therefore read in parallel under WAL. `release_memory()` closes every thread's
idle connection; the next read reopens it. `configure_connection` sets each connection to:

```sql
PRAGMA foreign_keys=ON;
PRAGMA journal_mode=WAL;
PRAGMA synchronous=NORMAL;
PRAGMA cache_size=-1024;          -- 1 MiB page cache per connection
PRAGMA mmap_size=0;
PRAGMA journal_size_limit=16777216;
```

It also sets a 128-entry statement cache and a busy handler, `wait_for_writer`, that waits up to
`BUSY_TIMEOUT` (250 ms) for the write lock on each attempt.

Every write runs under `retry_busy`: a `Busy` attempt is rolled back and re-run whole after a
jittered backoff that doubles from 2 ms to 200 ms. After `BUSY_DEADLINE` (10 s) of waiting the
write returns `StoreError::Busy`. An `OperationDeadline` makes all the writes of one operation (an
apply's publication passes, a snapshot's batches) share one deadline.

A workspace whose write ended `Busy` leaves the store alone for `STORE_BUSY_COOLDOWN` (30 s). Its
results stay in memory and the first apply after the cooldown publishes them. A workspace result's
`workspaceProfile` reports `storageRetries` and `storageRetryWaitNanoseconds`.

## Hashes, strings and paths

A `HashKey` is an algorithm and four signed 64-bit lanes, each eight digest bytes read with
`i64::from_be_bytes`. Algorithms are `HASH_SHA256` (1), `HASH_GIT_SHA1` (2) and `HASH_GIT_SHA256`
(3). A SHA-1 is zero-padded to 32 bytes. Test: `hash_lanes_are_stable_and_sha1_is_padded`.

`hashes(hash_id, algorithm_id, lane0..lane3)` stores each identity once, and every other table
references `hash_id`. Its only index is `hashes_by_prefix ON hashes(algorithm_id, lane0)`; lookups
use it and compare the other three lanes. There is no unique index on the full identity. Writers
insert a hash only where it does not exist yet, inside `BEGIN IMMEDIATE`, which serializes them.
Test: `hash_lookup_index_contains_only_algorithm_and_first_lane`.

`strings(domain_id, content_hash_id, collision_ordinal, value)` interns text per domain (path,
repository, owner, path component, clause key). Interning looks the text up by its SHA-256, compares
the stored value, and gives differing text with the same hash the next `collision_ordinal`. The text
itself is never indexed.

`paths` interns whole workspace-relative paths, so a path has one `path_id` across snapshots and
workspaces.

## Objects

`source_objects` holds each source once under its SHA-256, with its raw length, codec (1, Zstandard)
and frame. `tape_objects` holds one tape per source, `grammar_id`, `grammar_version` and
`tape_format_version`; the grammar version is the grammar build identity. Every frame is an
independent level-3 Zstandard frame, so one object is read without another. Both tables carry a
`created_generation` for collection.

`source_aliases` maps a Git blob ID to the source it names. Publication adds an alias when a file's
evaluated bytes hash to its committed blob, whoever supplied them (repository index or host edit),
and hashes only files whose path is clean in the worktree (`clean_blob_alias`). An alias that would
name different bytes fails the publication as corrupt.

Reads check the codec, the raw length and the SHA-256 against the row and fail with
`StoreError::Corrupt` on a mismatch. Test: `corrupt_zstandard_frames_fail_closed`. A stored tape
that does not load is parsed again.

### Documents of a file

A file can be several documents. Its primary document is its own tree over its bytes; an embedder
can derive more, such as Blade's compiled view or a Vue component's script program.

Every tape holds one source, one grammar and one tree (tape format 7, magic `TMPTAPE7`). A derived
document is stored as ordinary objects: its text is a `source_objects` row and its tree a
`tape_objects` row over that text, with the primary tape's grammar version and format. Its grammar
ID is the dialect's tape tag with bit 8 set and, above it, a fingerprint of the placeholders its
source map omits (0 for none). One text can therefore hold several derived tapes, never a primary's,
and derived documents with the same text and omissions share one (`document_grammar_id`).

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

The key is the primary tape, not a path: a file's documents follow from its source, grammar version
and tape format. Ordinal 0 is the primary and has no row, so a single-language file has none.
`parent_ordinal` names the document a row was derived from. `role_id` is 1 for an embedded document
(one byte-exact slice of its parent) and 2 for a generated one. `via_id` is the first eight bytes of
the SHA-256 of the embedding's registry name, read as a nonnegative integer (`via_code`).
`source_map` holds the persisted segments; `host_errors` holds the parent byte ranges the document
cannot be read through, as little-endian `u32` pairs.

Publication writes a new primary tape, its derived sources and tapes, and their rows in one
transaction; a tape already stored keeps the rows written with it. Loading reads a primary tape, its
rows and its derived objects in bulk (`file_documents_bulk`). If any part is missing or fails its
checks, the file is parsed again rather than restored without its derived documents. Each derived
tape gets its own search index, and a clause over a derived document consults that document's index.

## Git snapshots and overlays

Discovery (`native/src/git_workspace.rs`) reads repositories, indexes, attributes and worktree state
with `gix`; production code runs no `git` binary.

Discovery leaves out the built-in ignores (`BUILTIN_IGNORE`: `.git/`, `node_modules/`, `target/`,
`tmp/`, `__pycache__/`) and what `.rerailignore` names. `.rerailignore` uses Git ignore syntax with
negation; a negation cannot bring back a built-in ignore. These rules decide only which files the
store may restore, never which files a repository index evaluates. A gitlink (submodule) entry is
recorded as a nested path and not descended into.

`git_trees` and `git_tree_entries` store each Git tree's entries under interned names. A tree is
complete when its row has `complete=1` and still has the `entry_count` its publication wrote, so a
partial entry set is republished. Publication packs missing trees whole into transactions of up to
`GIT_TREE_BATCH_ENTRIES` (512) entries; a larger tree is written alone.

`snapshots` is unique on root tree, scanner version, grammar set and tape format, so workspaces on
the same commit share one snapshot; `snapshot_files` maps its paths to sources and tapes.
`publish_snapshot` registers the workspace in the snapshot's own transaction, and a snapshot needs a
published root tree (test: `snapshots_require_a_published_root_tree`).

`workspaces` records each workspace's root, repository (the Git common directory), owner
(`rerail:<pid>:<worktree>`), snapshot, lease and state (1 active, 2 retiring).
`workspace_overlay` maps a workspace's paths to the sources and tapes it evaluated.

A clean file's committed blob is its alias without reading the file (`clean_source_alias`). Before a
restore, `holds_blob_bytes` confirms the worktree still holds the blob's bytes: one `lstat` while
the stat matches the index entry, a hash otherwise. The conversions `filter`, `ident` and
`working-tree-encoding` always force the hash, since they can change bytes without changing the
size. Publication promotes clean files from the overlay into the snapshot
(`promote_overlay_files_to_snapshot`, at most 64 paths).

## Results and reuse

`candidate_catalogs` identifies a rule catalog: a SHA-256 over the code-rule catalog, grammar set,
scanner version and tape format (`CandidateCatalogIdentity`). `candidate_checks` names one clause of
a catalog by rule (policy hash) and clause key.

Per-file results are stored in `context_check_results(source_id, context_hash_id, check_id, codec_id,
raw_length, frame)`. The context (`file_context`) is everything besides content a file's results
depend on: its path, language, Python module and nearest `package.json`, plus any calls-closure
stamp (`result_context`). `local_check_results(source_id, check_id, ...)` is still in the schema and
collection still deletes from it, but nothing writes it. `LocalResultCache` keeps up to 32 MiB of
decoded results per process.

`completeStorageRevisionAsync()` records a completed index. In a Git worktree it writes
`completed_directory_reuse(tree_hash_id, prefix, package_hash_id, catalog_hash_id)` for each clean
tree, plus the workspace's row in `completed_workspace_revisions` (`complete_directories`). The
package hash covers what the tree's files read outside it: the Python package markers of its
ancestors and the `package.json` its ECMAScript files inherit. This write is a cache: when it fails
the audit keeps its results and loses only the reuse.

A repository index (`index.rs`) walks the files the host selects and asks which clean trees have a
completion record for the current catalog (`reusable_directory_prefixes`). Under those prefixes it
resolves clean files' aliases (`stored_sources_by_alias`) and their results
(`local_check_results_by_key`) in bulk. It restores a file only when it has results for every clause
and still holds its blob's bytes. Listing and restoring evaluate nothing. The rest is read in chunks
of at most `INDEX_BATCH_FILES` (32) files, stopping once a chunk holds `INDEX_BATCH_BYTES` (1 MiB).
Each chunk is published, and its sources and tapes are released, before it returns. The scan budgets
are in [Reference](reference.md#scan-and-settings-controls).

Result pages are not stored: `resultPageAsync()` pages an in-memory `ResultPageIndex`.

## Kind search index

The search index tells, without decoding a tape, whether a node kind occurs in it:

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

`search_kind_fts` is a contentless FTS5 table with one document per tape (`rowid` is the tape ID)
and one kind token per preorder node. It does not depend on the rule catalog. The trigger
`delete_tape_search_index` removes a tape's document when the tape is deleted.

The maintenance thread builds the index after publication, never during it. It reads tapes without
an index set through a keyset cursor (`tapes_without_search_index_after`), 64 at a time, for about
one second per wake. It writes each batch in one transaction (`put_search_indexes_bulk`). A tape this
build cannot decode gets an incomplete set and leaves the backlog.

`get_search_kind_presence_bulk` answers each lookup as `Present`, `Absent`, `SyntaxErrors` or
`Unknown`. Only a complete, error-free set at the requested index version can answer `Absent`; a
missing, stale or incomplete set is `Unknown`. When every AST clause's root kinds are `Absent`,
`restore_persisted_file` skips decoding the tape and records no witnesses.

## Overlay publication

`publish_overlay_files` publishes 1 to `MAX_PUBLICATION_BATCH` (64) files of one workspace. It takes
no write lock for work it can do outside one:

1. It validates each file and hashes its source and tape on the worker pool.
2. In one deferred read transaction it looks up the stored sources and tapes, paths, catalogs and
   checks (three bulk statements).
3. It compresses only the sources and tapes the store lacks.

If anything is missing, an object pass writes it under `BEGIN IMMEDIATE`. Each statement takes the
whole batch: identities as a JSON parameter, frames as bound BLOBs. Each joins rows by full identity,
never by an ID read earlier. The statements insert, in order: hashes, strings, paths, catalogs,
checks, sources, aliases, tapes and derived documents.

A pass that would run more than ten data statements fails. After the pass a new read checks that
every row exists. A collection between the read and the write can delete rows the pass relied on;
the next pass writes them again. Publication fails after eight passes without completing.

The relationship pass then writes the overlay rows, any new result-context hashes, and
`context_check_results`, chunked to SQLite's variable limit, again in at most ten statements. Under
the write lock it first checks that every path, source, tape, catalog and check ID it prepared is
still current (`relationship_references_current`). If one is not, the whole publication restarts,
at most eight times.

Every pass runs under `retry_busy`. Cancellation is checked between passes and before the commit;
an uncommitted pass rolls back and committed passes stay valid. Publication returns its statement,
pass, retry and timing counts in `PublicationResult`. Tests: the ten-statement budget in
`overlay_publication_replaces_payload_and_candidates_atomically`, no hashing under the write lock in
`relationship_revalidation_does_not_hash_payloads_under_write_lock`, and the reconciliation tests
`reconciliation_reprepares_source_collected_after_the_read` and
`reconciliation_revalidates_path_collected_before_relationship_write`.

Publication does not make results reusable. A repository index restores only under a completion
record (above), written after the whole index is published.

## Maintenance and collection

Each process keeps one `ContentStore` and one maintenance thread (`rerail-store-maintenance`) per
store path (`shared_content_store`). The thread wakes every 30 seconds and does three things:

- It renews the leases of its registered workspaces to 120 seconds ahead, in one statement.
- It runs one collection pass when the store's capacity state is not `Normal`.
- It builds search indexes.

`capacity()` sums the database file and its `-wal` and `-shm` files. The thresholds are
`STORE_SOFT_LIMIT` (7 GiB, `Collect`), `STORE_HARD_LIMIT` (9 GiB, `RejectWrites`) and
`STORE_ABSOLUTE_LIMIT` (10 GiB, `OverBudget`). Below 7 GiB nothing is collected. From 9 GiB new
sources, tapes and search indexes are refused. Test: `capacity_thresholds_are_exact`.

Database relationships are the only liveness authority; there are no reference counts.
`collect(now, batch)` takes at most `MAX_COLLECTION_BATCH` (256) rows per statement and runs one
`BEGIN IMMEDIATE` transaction in this order:

1. Expired `object_pins`.
2. Workspaces whose lease expired become retiring; their overlay rows go, then the workspace and its
   `completed_workspace_revisions` row.
3. The files of one snapshot no workspace references, then the snapshot once it is empty.
4. Tapes no snapshot file, overlay, pin or `file_documents.document_tape_id` references. A deleted
   primary tape takes its `file_documents` rows, search index set and FTS document with it, so a
   later pass deletes the derived tapes no row names.
5. Unreferenced sources, after their aliases and stored results.
6. Checks without results, catalogs without checks, and `completed_directory_reuse` rows whose
   catalog is gone.
7. Git trees no snapshot reaches, each deleted whole with its entries, within the
   `GIT_TREE_BATCH_ENTRIES` budget (always at least one tree).
8. Unreferenced paths, strings and hashes.

Collection deletes only objects whose `created_generation` is older than the store's generation,
then increments it. An object published since the last pass therefore survives this one. After the
commit it runs `PRAGMA wal_checkpoint(PASSIVE)` and `PRAGMA incremental_vacuum(128)` and clears the
process's caches.

Every reference probe uses an index; test `collect_statements_never_scan_inside_correlated_subqueries`
checks each statement's query plan. A closed workspace is no longer renewed, so its lease runs out
and collection retires it. `retire_workspace` and the pin API (`pin_source`, `pin_tape`,
`release_pins`) exist but have no production caller.

## Memory

- The decompressed read cache holds up to 32 MiB of sources and tapes with a 30-second sliding
  lifetime (`DEFAULT_READ_CACHE_BYTES`, `DEFAULT_DECOMPRESSED_CACHE_TTL`). Entries live in one
  anonymous mapping twice that size.
- When the mapping fills, live entries are copied into a replacement and the old mapping stays
  readable until its readers drop it. There are at most two mappings, and each gets `MADV_DONTNEED`
  before `munmap`. An entry larger than the limit, or one that arrives while a retired mapping is
  still in use, is kept on the heap.
- `LocalResultCache` holds up to 32 MiB of decoded results.
- Each connection's SQLite page cache is 1 MiB.
- `release_memory()` clears both caches and closes idle connections.
