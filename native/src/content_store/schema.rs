//! The content store's SQLite schema: one current, idempotent DDL.

use super::connection::configure_connection;
use super::{ContentStore, StoreError, StoreResult};
#[allow(unused_imports)]
use crate::content_store::connection::Cached as _;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::collections::HashSet;
use std::sync::LazyLock;

// `user_version` is the compatibility floor: a binary refuses a store whose floor is newer than
// its SCHEMA_VERSION, so the floor moves only when an older binary would corrupt or misread the
// store. An open that finds any SCHEMA object missing applies SCHEMA, which is idempotent, so
// additive changes that older SQL ignores, such as indexes, need no floor raise: older binaries
// tolerate them.
//
// The managed installation's store is one file per machine (`~/.cache/rerail/`), shared by
// every installed version; a build running from a checkout keeps its own in `<checkout>/.rerail/`.
// Raising the floor therefore also renames the file (STORE_FILE_NAME), wherever it lives: older
// binaries keep their own store and a new store starts empty, since the store is a cache. The
// host joins the name (exported as `contentStoreFileName`) to its build's directory. A store is
// never upgraded: the old file is neither migrated nor deleted, since an older binary may still
// run against it. It is safe to delete by hand once no older binary runs; the CHANGELOG entry of
// each floor raise names the file (with its -wal and -shm). STORE_FILE_FLOOR pins the pairing, so
// raising SCHEMA_VERSION alone does not compile.
pub(super) const SCHEMA_VERSION: i64 = 9;

/// File name of every content store, managed or checkout-private, renamed whenever the floor rises.
pub const STORE_FILE_NAME: &str = "content-v9.sqlite";
/// The floor `STORE_FILE_NAME` was introduced at. 9: a space's kept facts are a chain of packed files (decision 0456),
/// and there are no frames (decision 0439), which an older binary would read as a store that keeps nothing, and a
/// kept column may be a node id split at its granule, which it would read as wrong values (7 on
/// final-rerail-shape-rr-align, joined with 8 and 7 below on sync-integration-hub-tape); 8: every check result, call
/// summary and tree page is in a segment, as it is, its row naming its range (decision 0450 step 2, 2026-11-19), which
/// an older binary would read as a row with no frame; 7: every source and tape is in a segment, as it is, and no row
/// holds an object's bytes (decision 0295 step 5, 2026-11-19), which an older binary would read as an empty or corrupt
/// frame; 6: the atoms are blocks in files beside the store (decision 0401:
/// `atom_blocks`), which an older binary would read as a store with no atoms; 5: a frame's row points at its bytes in a segment beside the store
/// (decision 0295 §4: `frames.seg`), which an older binary would read as an empty frame; 4: a corpus manifest's lane is a column of it
/// (`corpus_manifests.lane`), not a table of its own (decision 0104); 3 was atoms stored as segments
/// of the arena (decision 0049); 2 the prefilter vocabulary keyed by atom.
const STORE_FILE_FLOOR: i64 = 9;
const _: () = assert!(
    SCHEMA_VERSION == STORE_FILE_FLOOR,
    "raising the floor requires a new STORE_FILE_NAME"
);

/// The current schema. Idempotent: every open applies it, creating what a store lacks.
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS hashes (
    hash_id INTEGER PRIMARY KEY,
    algorithm_id INTEGER NOT NULL,
    lane0 INTEGER NOT NULL,
    lane1 INTEGER NOT NULL,
    lane2 INTEGER NOT NULL,
    lane3 INTEGER NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS hashes_by_prefix ON hashes(algorithm_id, lane0);
CREATE TABLE IF NOT EXISTS strings (
    string_id INTEGER PRIMARY KEY,
    domain_id INTEGER NOT NULL,
    content_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    collision_ordinal INTEGER NOT NULL DEFAULT 0,
    value TEXT NOT NULL,
    UNIQUE (domain_id, content_hash_id, collision_ordinal)
) STRICT;
CREATE INDEX IF NOT EXISTS strings_by_hash ON strings(domain_id, content_hash_id);
CREATE INDEX IF NOT EXISTS strings_by_content_hash ON strings(content_hash_id);
CREATE TABLE IF NOT EXISTS paths (
    path_id INTEGER PRIMARY KEY,
    string_id INTEGER NOT NULL UNIQUE REFERENCES strings(string_id)
) STRICT;
CREATE TABLE IF NOT EXISTS source_objects (
    source_id INTEGER PRIMARY KEY,
    canonical_hash_id INTEGER NOT NULL UNIQUE REFERENCES hashes(hash_id),
    raw_length INTEGER NOT NULL,
    codec_id INTEGER NOT NULL,
    frame BLOB NOT NULL,
    created_generation INTEGER NOT NULL,
    seg INTEGER NOT NULL DEFAULT 0,
    seg_at INTEGER NOT NULL DEFAULT 0,
    seg_len INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE TABLE IF NOT EXISTS source_aliases (
    alias_hash_id INTEGER PRIMARY KEY REFERENCES hashes(hash_id),
    source_id INTEGER NOT NULL REFERENCES source_objects(source_id)
) STRICT;
CREATE INDEX IF NOT EXISTS source_aliases_by_source ON source_aliases(source_id);
CREATE TABLE IF NOT EXISTS tape_objects (
    tape_id INTEGER PRIMARY KEY,
    source_id INTEGER NOT NULL REFERENCES source_objects(source_id),
    canonical_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    grammar_id INTEGER NOT NULL,
    grammar_version INTEGER NOT NULL,
    tape_format_version INTEGER NOT NULL,
    codec_id INTEGER NOT NULL,
    raw_length INTEGER NOT NULL,
    frame BLOB NOT NULL,
    created_generation INTEGER NOT NULL,
    seg INTEGER NOT NULL DEFAULT 0,
    seg_at INTEGER NOT NULL DEFAULT 0,
    seg_len INTEGER NOT NULL DEFAULT 0,
    -- The file's programs (decision 0431 step 6): the number of its `file_documents` rows, written in the transaction
    -- that writes them; -1 where not known (a tape stored without its programs).
    programs INTEGER NOT NULL DEFAULT -1,
    UNIQUE (source_id, grammar_id, grammar_version, tape_format_version)
) STRICT;
-- The segments sources and tapes are in (decision 0295 step 5: `<store>.objects/<seg>.seg`, mapped, uncompressed): every
-- row is of codec 0, names its range (`seg`, `seg_at`, `seg_len`) and holds no frame (floor 7: `frame` is always
-- empty); `seg` -1 is a row whose segment went missing (absent until stored again).
CREATE TABLE IF NOT EXISTS object_segments(seg INTEGER PRIMARY KEY);
CREATE INDEX IF NOT EXISTS tape_objects_by_hash ON tape_objects(canonical_hash_id);
-- The documents a file derives from its own (an embedded program, a generated one), keyed by the
-- primary document's tape: documents are a function of source, grammar version and tape format,
-- not of a path. Each derived document's text is an ordinary source and its tape an ordinary
-- tape row; a row keeps that tape (and so its source) alive and dies with the primary tape.
-- Ordinal 0 is the primary itself and has no row; a single-language file has none.
CREATE TABLE IF NOT EXISTS file_documents (
    tape_id INTEGER NOT NULL REFERENCES tape_objects(tape_id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK(ordinal > 0),
    parent_ordinal INTEGER NOT NULL CHECK(parent_ordinal >= 0 AND parent_ordinal < ordinal),
    role_id INTEGER NOT NULL CHECK(role_id > 0),
    via_id INTEGER NOT NULL,
    document_tape_id INTEGER NOT NULL REFERENCES tape_objects(tape_id),
    source_map BLOB NOT NULL,
    host_errors BLOB NOT NULL,
    -- The parent's bytes the document stands at, where they are not its text's (`tape::DerivedImage::at`).
    at_start INTEGER,
    at_end INTEGER,
    PRIMARY KEY (tape_id, ordinal)
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS file_documents_by_document ON file_documents(document_tape_id);

-- A tree is complete only when its row says so and it still has the entry count its publication
-- wrote, so a partial entry set is republished whichever writer left it.
CREATE TABLE IF NOT EXISTS git_trees (
    tree_id INTEGER PRIMARY KEY,
    tree_hash_id INTEGER NOT NULL UNIQUE REFERENCES hashes(hash_id),
    complete INTEGER NOT NULL DEFAULT 0 CHECK(complete IN (0,1)),
    entry_count INTEGER
) STRICT;
CREATE TABLE IF NOT EXISTS git_tree_entries (
    tree_id INTEGER NOT NULL REFERENCES git_trees(tree_id),
    name_string_id INTEGER NOT NULL REFERENCES strings(string_id),
    mode_id INTEGER NOT NULL,
    object_kind_id INTEGER NOT NULL,
    target_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    PRIMARY KEY (tree_id, name_string_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS git_tree_entries_by_name ON git_tree_entries(name_string_id);
CREATE INDEX IF NOT EXISTS git_tree_entries_by_target ON git_tree_entries(target_hash_id);
CREATE TABLE IF NOT EXISTS snapshots (
    snapshot_id INTEGER PRIMARY KEY,
    root_tree_id INTEGER NOT NULL REFERENCES git_trees(tree_id),
    scanner_version INTEGER NOT NULL,
    grammar_set_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    tape_format_version INTEGER NOT NULL,
    UNIQUE (root_tree_id, scanner_version, grammar_set_hash_id, tape_format_version)
) STRICT;
CREATE INDEX IF NOT EXISTS snapshots_by_grammar_set ON snapshots(grammar_set_hash_id);
CREATE TABLE IF NOT EXISTS snapshot_files (
    snapshot_id INTEGER NOT NULL REFERENCES snapshots(snapshot_id),
    path_id INTEGER NOT NULL REFERENCES paths(path_id),
    source_id INTEGER NOT NULL REFERENCES source_objects(source_id),
    tape_id INTEGER REFERENCES tape_objects(tape_id),
    PRIMARY KEY (snapshot_id, path_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS snapshot_files_by_source ON snapshot_files(source_id);
CREATE INDEX IF NOT EXISTS snapshot_files_by_tape ON snapshot_files(tape_id) WHERE tape_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS snapshot_files_by_path ON snapshot_files(path_id);
CREATE TABLE IF NOT EXISTS workspaces (
    workspace_id INTEGER PRIMARY KEY,
    root_path_id INTEGER NOT NULL UNIQUE REFERENCES paths(path_id),
    repository_string_id INTEGER NOT NULL REFERENCES strings(string_id),
    owner_string_id INTEGER NOT NULL REFERENCES strings(string_id),
    snapshot_id INTEGER NOT NULL REFERENCES snapshots(snapshot_id),
    generation INTEGER NOT NULL,
    lease_until INTEGER NOT NULL,
    state_id INTEGER NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS workspaces_by_lease ON workspaces(state_id, lease_until);
CREATE INDEX IF NOT EXISTS workspaces_by_repository ON workspaces(repository_string_id);
CREATE INDEX IF NOT EXISTS workspaces_by_owner ON workspaces(owner_string_id);
CREATE INDEX IF NOT EXISTS workspaces_by_snapshot ON workspaces(snapshot_id);
-- Store sessions (decision 0450 §2): the durable owners whose latest data is live. `kind` 1 is a workspace, `key` its
-- root (`paths`); 2 a corpus lane, `key` a digest of its root and lane, its row written by each keep decision; 3 a pin's
-- owner, `key` its name (`strings`), its source and tape pins its data. `latest` is its latest generation, `last_used` the unix second it was last registered or renewed,
-- `ended` the second its lease ended (NULL while something keeps it alive), and `aged_bytes` the bytes its data held
-- then: an ended session's data is aged under one budget and backstop, the oldest-ended released first (0450 step 3).
CREATE TABLE IF NOT EXISTS store_sessions (
    kind INTEGER NOT NULL,
    key INTEGER NOT NULL,
    latest INTEGER NOT NULL,
    last_used INTEGER NOT NULL,
    ended INTEGER,
    aged_bytes INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (kind, key)
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS store_sessions_by_ended ON store_sessions(kind, ended) WHERE ended IS NOT NULL;
CREATE TABLE IF NOT EXISTS analysis_scope_snapshot (
    scope TEXT NOT NULL,
    snapshot_id INTEGER NOT NULL REFERENCES snapshots(snapshot_id) ON DELETE CASCADE,
    acquired_at INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(scope, snapshot_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS analysis_scope_snapshot_by_snapshot ON analysis_scope_snapshot(snapshot_id);
CREATE TABLE IF NOT EXISTS analysis_scope_generation (
    scope TEXT PRIMARY KEY,
    generation INTEGER NOT NULL
) STRICT, WITHOUT ROWID;
-- Only a fully observed directory generation has a known maximum control generation.
-- The separate table preserves the two-column INSERT used by older writers.
CREATE TABLE IF NOT EXISTS analysis_directory_acquisition (
    scope TEXT PRIMARY KEY REFERENCES analysis_scope_generation(scope) ON DELETE CASCADE,
    max_control_generation INTEGER NOT NULL
) STRICT, WITHOUT ROWID;
CREATE TRIGGER IF NOT EXISTS analysis_directory_acquisition_unknown BEFORE INSERT ON analysis_scope_generation
WHEN NEW.scope GLOB 'cwd:*' BEGIN
    DELETE FROM analysis_directory_acquisition WHERE scope=NEW.scope;
END;
CREATE TABLE IF NOT EXISTS analysis_scope_workspace_snapshot (
    scope TEXT NOT NULL,
    workspace TEXT NOT NULL,
    snapshot_id INTEGER NOT NULL REFERENCES snapshots(snapshot_id) ON DELETE CASCADE,
    acquired_at INTEGER NOT NULL DEFAULT 0,
    acquisition INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(scope, workspace)
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS analysis_scope_workspace_snapshot_by_snapshot ON analysis_scope_workspace_snapshot(snapshot_id);
CREATE TABLE IF NOT EXISTS analysis_scope_collect (
    snapshot_id INTEGER PRIMARY KEY REFERENCES snapshots(snapshot_id) ON DELETE CASCADE
) STRICT;
CREATE TABLE IF NOT EXISTS analysis_scope_pending (
    id TEXT PRIMARY KEY
) STRICT, WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS analysis_scope_release (
    id TEXT PRIMARY KEY,
    scope TEXT NOT NULL DEFAULT '',
    before INTEGER NOT NULL DEFAULT 0,
    legacy_scope TEXT,
    legacy_acquisition INTEGER
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS analysis_scope_release_by_scope ON analysis_scope_release(scope,before);
CREATE INDEX IF NOT EXISTS analysis_scope_release_legacy ON analysis_scope_release(legacy_scope,before) WHERE legacy_scope IS NOT NULL;
-- A compacted child receipt can still arrive through an older writer. Ignore its insert before
-- its delivery captures a new acquisition number or releases memberships. Probe the cutoff
-- index at each path boundary, including a root that ends in the separator.
CREATE TRIGGER IF NOT EXISTS analysis_skip_covered_directory_stop BEFORE INSERT ON analysis_scope_release
WHEN NEW.scope<>'' AND EXISTS (
    SELECT 1 FROM analysis_scope_release WHERE before>=NEW.before AND scope IN (RR_NEW_DIRECTORY_ANCESTORS)
) BEGIN SELECT RAISE(IGNORE); END;
-- A previous writer deletes a lower exact cutoff before inserting its replacement. If an
-- ancestor will suppress the insert, retain any later acquisition boundary the child carries.
CREATE TRIGGER IF NOT EXISTS analysis_keep_legacy_directory_fence BEFORE DELETE ON analysis_scope_release
WHEN OLD.legacy_acquisition > (
    SELECT max(legacy_acquisition) FROM analysis_scope_release WHERE id<>OLD.id AND before>=OLD.before
        AND scope IN (RR_OLD_DIRECTORY_ANCESTORS)
) BEGIN SELECT RAISE(IGNORE); END;
CREATE TABLE IF NOT EXISTS analysis_history_scope (
    scope TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    stop_scopes TEXT NOT NULL
) STRICT, WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS analysis_history_ancestor (
    ancestor TEXT NOT NULL,
    scope TEXT NOT NULL REFERENCES analysis_history_scope(scope) ON DELETE CASCADE,
    PRIMARY KEY(ancestor,scope)
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS analysis_history_ancestor_by_scope ON analysis_history_ancestor(scope);
INSERT OR IGNORE INTO analysis_history_ancestor(ancestor,scope)
    SELECT a.value,h.scope FROM analysis_history_scope h, json_each(h.stop_scopes) a;
CREATE TRIGGER IF NOT EXISTS analysis_history_index_scope AFTER INSERT ON analysis_history_scope BEGIN
    INSERT OR IGNORE INTO analysis_history_ancestor(ancestor,scope) SELECT value,NEW.scope FROM json_each(NEW.stop_scopes);
END;
CREATE TABLE IF NOT EXISTS analysis_scope_run (
    scope TEXT NOT NULL REFERENCES analysis_history_scope(scope),
    acquisition INTEGER NOT NULL,
    acquired_at INTEGER NOT NULL,
    scan_id TEXT NOT NULL,
    directory TEXT,
    repository TEXT,
    tree TEXT,
    git_commit TEXT,
    PRIMARY KEY(scope,acquisition)
) STRICT, WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS analysis_run_workspace_snapshot (
    scope TEXT NOT NULL,
    acquisition INTEGER NOT NULL,
    snapshot_id INTEGER NOT NULL REFERENCES snapshots(snapshot_id) ON DELETE CASCADE,
    PRIMARY KEY(scope,acquisition,snapshot_id),
    FOREIGN KEY(scope,acquisition) REFERENCES analysis_scope_run(scope,acquisition) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS analysis_run_workspace_by_snapshot ON analysis_run_workspace_snapshot(snapshot_id);
-- These triggers also run when an older binary delivers a stop.
CREATE TRIGGER IF NOT EXISTS analysis_runs_end_generation AFTER UPDATE OF generation ON analysis_scope_generation
WHEN NEW.generation != OLD.generation BEGIN
    DELETE FROM analysis_scope_run WHERE scope=NEW.scope;
END;
-- Reinstall this trigger on upgrade. Earlier stores enumerated all scopes for each delivery.
DROP TRIGGER IF EXISTS analysis_runs_stop_directory;
CREATE TRIGGER IF NOT EXISTS analysis_runs_stop_directory AFTER INSERT ON analysis_scope_release
WHEN NEW.scope != '' BEGIN
    DELETE FROM analysis_scope_run WHERE acquired_at<=NEW.before AND scope IN (
        SELECT scope FROM analysis_history_ancestor WHERE ancestor=NEW.scope
        UNION ALL
        SELECT a.scope FROM analysis_history_ancestor a JOIN analysis_history_scope h USING(scope)
        WHERE a.ancestor='cwd:'||substr(NEW.scope,5+instr(substr(NEW.scope,5),':'))
            AND instr(substr(NEW.id,instr(NEW.id,':')+1),':')>0 AND h.scope='cwd:'||h.value
    );
END;
CREATE TRIGGER IF NOT EXISTS analysis_history_forget_empty AFTER DELETE ON analysis_scope_run BEGIN
    DELETE FROM analysis_history_scope WHERE scope=OLD.scope
        AND NOT EXISTS (SELECT 1 FROM analysis_scope_run WHERE scope=OLD.scope);
END;
CREATE TABLE IF NOT EXISTS workspace_overlay (
    workspace_id INTEGER NOT NULL REFERENCES workspaces(workspace_id),
    path_id INTEGER NOT NULL REFERENCES paths(path_id),
    state_id INTEGER NOT NULL,
    source_id INTEGER REFERENCES source_objects(source_id),
    tape_id INTEGER REFERENCES tape_objects(tape_id),
    PRIMARY KEY (workspace_id, path_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS workspace_overlay_by_source ON workspace_overlay(source_id) WHERE source_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS workspace_overlay_by_tape ON workspace_overlay(tape_id) WHERE tape_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS workspace_overlay_by_path ON workspace_overlay(path_id);
CREATE TABLE IF NOT EXISTS object_pins (
    object_kind_id INTEGER NOT NULL,
    object_id INTEGER NOT NULL,
    owner_string_id INTEGER NOT NULL REFERENCES strings(string_id),
    lease_until INTEGER NOT NULL,
    PRIMARY KEY (object_kind_id, object_id, owner_string_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS object_pins_by_lease ON object_pins(lease_until);
CREATE INDEX IF NOT EXISTS object_pins_by_owner ON object_pins(owner_string_id);
CREATE TABLE IF NOT EXISTS store_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    generation INTEGER NOT NULL,
    analysis_acquisitions INTEGER NOT NULL DEFAULT 0
) STRICT;
INSERT OR IGNORE INTO store_state(singleton, generation) VALUES (1, 1);

-- Older writers also insert zero then read back the generation. Keep the trigger names
-- they expect, and seed both named and directory scopes from the store-wide sequence.
DROP TRIGGER IF EXISTS analysis_named_generation_begin;
CREATE TRIGGER IF NOT EXISTS analysis_named_generation_begin AFTER INSERT ON analysis_scope_generation
WHEN (NEW.scope GLOB 'named:*' OR NEW.scope GLOB 'cwd:*') AND NEW.generation=0 BEGIN
    UPDATE store_state SET analysis_acquisitions=analysis_acquisitions+1 WHERE singleton=1;
    UPDATE analysis_scope_generation SET generation=(SELECT analysis_acquisitions FROM store_state WHERE singleton=1) WHERE scope=NEW.scope;
END;
DROP TRIGGER IF EXISTS analysis_named_generation_end;
CREATE TRIGGER IF NOT EXISTS analysis_named_generation_end BEFORE DELETE ON analysis_scope_generation
WHEN OLD.scope GLOB 'named:*' OR OLD.scope GLOB 'cwd:*' BEGIN
    UPDATE store_state SET analysis_acquisitions=max(analysis_acquisitions,OLD.generation) WHERE singleton=1;
END;

-- A home-qualified stop also covers unqualified owners acquired by the time it reaches this store.
-- Capture that boundary for deliveries from older binaries too. Replays do not insert a receipt.
CREATE TRIGGER IF NOT EXISTS analysis_scope_legacy_stop AFTER INSERT ON analysis_scope_release
WHEN NEW.scope<>'' AND instr(substr(NEW.id,instr(NEW.id,':')+1),':')>0 BEGIN
    UPDATE analysis_scope_release SET legacy_scope='cwd:'||substr(NEW.scope,5+instr(substr(NEW.scope,5),':')),
        legacy_acquisition=(SELECT analysis_acquisitions FROM store_state WHERE singleton=1) WHERE id=NEW.id;
END;
-- Reclaim directory generations at first delivery, including deliveries by an older binary.
-- Replaying a home stop must leave a later, unqualified writer with its original generation.
CREATE TRIGGER IF NOT EXISTS analysis_release_directory_generation AFTER INSERT ON analysis_scope_release
WHEN NEW.scope<>'' BEGIN
    DELETE FROM analysis_scope_generation WHERE scope IN (
        WITH stops(root) AS (
            SELECT NEW.scope UNION ALL SELECT 'cwd:'||substr(NEW.scope,5+instr(substr(NEW.scope,5),':'))
            WHERE instr(substr(NEW.id,instr(NEW.id,':')+1),':')>0
        ), bounds(root,prefix) AS (
            SELECT root,CASE WHEN substr(root,-1)='RR_PATH_SEPARATOR' THEN root ELSE root||'RR_PATH_SEPARATOR' END FROM stops
        )
        SELECT a.scope FROM bounds b JOIN analysis_directory_acquisition a
            ON a.scope=b.root OR (a.scope>=b.prefix AND a.scope<(substr(b.prefix,1,length(b.prefix)-1)||char(unicode(substr(b.prefix,-1))+1)))
            WHERE a.max_control_generation<=NEW.before
    );
END;
UPDATE analysis_scope_release SET legacy_scope='cwd:'||substr(scope,5+instr(substr(scope,5),':')),
    legacy_acquisition=(SELECT analysis_acquisitions FROM store_state WHERE singleton=1)
    WHERE legacy_acquisition IS NULL AND scope<>'' AND instr(substr(id,instr(id,':')+1),':')>0;

-- Which store this file is: random when it is created. What a process holds of a store by its path (its atoms,
-- atoms.rs; its threads' connections, connection.rs) belongs to the store of this identity only, so a store
-- deleted and created again at the same path, by any process, is not read with the old one's. Additive: older
-- binaries never read it.
CREATE TABLE IF NOT EXISTS store_identity (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    id INTEGER NOT NULL
) STRICT;
INSERT OR IGNORE INTO store_identity(singleton, id) VALUES (1, random());

-- A workspace's results, their catalogs and checks, its completed directories and revisions are entries of the
-- store's per-file memo (`results.rs`, t33 step 8): a store made before loses the tables.
DROP TABLE IF EXISTS context_check_results;
DROP TABLE IF EXISTS candidate_checks;
DROP TABLE IF EXISTS candidate_catalogs;
DROP TABLE IF EXISTS completed_directory_reuse;
DROP TABLE IF EXISTS completed_workspace_revisions;

-- A query-independent search index. The kind stream contains one token per preorder node, so an
-- FTS5 instance offset is the preorder ordinal; terminal transducers confirm its candidates.
-- Contentless FTS stores no second copy of source. A tape with syntax errors must never be
-- skipped undecoded, so its index records them.
CREATE TABLE IF NOT EXISTS search_index_sets (
    tape_id INTEGER PRIMARY KEY REFERENCES tape_objects(tape_id) ON DELETE CASCADE,
    index_version INTEGER NOT NULL CHECK(index_version >= 0),
    node_count INTEGER NOT NULL CHECK(node_count > 0),
    complete INTEGER NOT NULL CHECK(complete IN (0,1)),
    has_errors INTEGER NOT NULL DEFAULT 0 CHECK(has_errors IN (0,1))
) STRICT;
CREATE VIRTUAL TABLE IF NOT EXISTS search_kind_fts USING fts5(
    token, content='', contentless_delete=1, detail=full, tokenize='ascii'
);
-- Physical rule planning reads exact term frequencies from SQL instead of estimating them in Rust.
CREATE VIRTUAL TABLE IF NOT EXISTS search_kind_vocab USING fts5vocab(search_kind_fts, 'row');
CREATE TRIGGER IF NOT EXISTS delete_tape_search_index BEFORE DELETE ON tape_objects BEGIN
    DELETE FROM search_kind_fts WHERE rowid=OLD.tape_id;
END;

-- The facts engine's memo (facts_results, its completions and plan sets) went (t33 step 3): its users keep their
-- entries as kept facts of the store (`kept_facts::HOLDER`), and a store made before loses the tables.
DROP INDEX IF EXISTS facts_results_by_plan;
DROP INDEX IF EXISTS facts_completions_by_set;
DROP TABLE IF EXISTS facts_results;
DROP TABLE IF EXISTS facts_completions;
DROP TABLE IF EXISTS facts_plan_sets;
-- Neural Engine models compiled on this machine (decision 0098 §3) are kept facts of the store, their archives
-- sources (`accel_models.rs`, t33 step 6): a store made before loses the table.
DROP TABLE IF EXISTS accel_models;

-- The cross-file memo's tables (decision 0076), removed with it (decision 0450 §3): a store made before loses them
-- when SCHEMA is next applied.
DROP TABLE IF EXISTS cross_blocks;
DROP TABLE IF EXISTS cross_rows;
DROP TABLE IF EXISTS cross_components;

-- Corpus runs' roots (decision 0053, corpus_runs.rs). Each run of a root records a manifest: the sources it read
-- (delta-varint ids, sorted), the grammar version and tape format of their tapes, and its memo plan. A run whose
-- manifest matches an earlier one of its root only moves that one's `last_run`. `corpus_live` is the union of the
-- kept manifests' (source, grammar version, tape format): collection keeps those sources and tapes. A plan in
-- `corpus_plans` that no kept manifest names (`live` 0) has its memo rows collected. Additive: an older binary's
-- collection ignores these tables, which costs a corpus run a cache miss, never a wrong result.
CREATE TABLE IF NOT EXISTS corpus_roots (
    root_id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE
) STRICT;
CREATE TABLE IF NOT EXISTS corpus_manifests (
    manifest_id INTEGER PRIMARY KEY,
    root_id INTEGER NOT NULL REFERENCES corpus_roots(root_id),
    last_run INTEGER NOT NULL,
    last_time INTEGER NOT NULL,
    grammar_version INTEGER NOT NULL,
    tape_format INTEGER NOT NULL,
    plan_digest BLOB NOT NULL CHECK(length(plan_digest) = 32),
    digest BLOB NOT NULL CHECK(length(digest) = 32),
    sources BLOB NOT NULL,
    -- What the run was of (`record_corpus_run_lane`): a root's last runs are protected by lane. NULL: the root's one
    -- lane.
    lane BLOB CHECK(lane IS NULL OR length(lane) = 32),
    -- The lane's policy set (`record_corpus_run_policy`, decision 0302): a set's newer lane supersedes its older. NULL:
    -- a lane of no set. Additive (`ADDED_COLUMNS`): an older binary ignores it.
    policy BLOB CHECK(policy IS NULL OR length(policy) = 32),
    UNIQUE (root_id, digest)
) STRICT;
CREATE INDEX IF NOT EXISTS corpus_manifests_by_run ON corpus_manifests(last_run);
CREATE TABLE IF NOT EXISTS analysis_scope_manifest (
    scope TEXT NOT NULL,
    manifest_id INTEGER NOT NULL REFERENCES corpus_manifests(manifest_id) ON DELETE CASCADE,
    acquired_at INTEGER NOT NULL DEFAULT 0,
    acquisition INTEGER NOT NULL DEFAULT 0,
    recorded_run INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(scope, manifest_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS analysis_scope_manifest_by_manifest ON analysis_scope_manifest(manifest_id);
CREATE TABLE IF NOT EXISTS corpus_live (
    source_id INTEGER NOT NULL,
    grammar_version INTEGER NOT NULL,
    tape_format INTEGER NOT NULL,
    PRIMARY KEY (source_id, grammar_version, tape_format)
) STRICT, WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS corpus_plans (
    plan_digest BLOB PRIMARY KEY CHECK(length(plan_digest) = 32),
    live INTEGER NOT NULL
) STRICT, WITHOUT ROWID;
-- Per memo plan, the relations its entries hold rows of, with their declared columns as a JSON array of
-- [name, type] (docs/sql-access/01-design.md §2): a session reads a run's entries by name. Collected with the plan.
CREATE TABLE IF NOT EXISTS corpus_plan_relations (
    plan_digest BLOB NOT NULL CHECK(length(plan_digest) = 32),
    relation TEXT NOT NULL,
    columns TEXT NOT NULL,
    PRIMARY KEY (plan_digest, relation)
) STRICT, WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS corpus_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    run INTEGER NOT NULL,
    history_bytes INTEGER NOT NULL,
    due INTEGER NOT NULL,
    archive_due INTEGER NOT NULL DEFAULT 0
) STRICT;
INSERT OR IGNORE INTO corpus_state(singleton, run, history_bytes, due) VALUES (1, 0, 0, 0);
-- The archive (decision 0450 step 4, `archive.rs`): an aged source's or tape's bytes as one zstd frame, its row's `seg`
-- -2 (its `seg_len` its length). `kind` is `object_pins`' object kind (1 a tape, 2 a source). Placed back in a segment
-- when read again. `corpus_state.archive_due`: set when a session ends or a keep decision is made, cleared by a pass
-- that archives nothing.
CREATE TABLE IF NOT EXISTS object_archive (
    kind INTEGER NOT NULL,
    id INTEGER NOT NULL,
    frame BLOB NOT NULL,
    PRIMARY KEY (kind, id)
) STRICT, WITHOUT ROWID;
-- The last keep decision: the run counter and unix second it was made at. A new manifest makes a decision due, but
-- a pass makes one only when it can change something (`CorpusPolicy`): a root past its protected runs since, or
-- enough runs or seconds since. A new root's first runs are protected, so a run of fresh roots decides rarely.
-- Additive: an older binary ignores it. `swept`: the decision whose unreachable memo entries were last collected (decision
-- 0227); the memo sweep is due while `run` is past it (`ADDED_COLUMNS` adds it to an older store).
CREATE TABLE IF NOT EXISTS corpus_decided (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    run INTEGER NOT NULL,
    time INTEGER NOT NULL,
    swept INTEGER NOT NULL DEFAULT 0
) STRICT;
INSERT OR IGNORE INTO corpus_decided(singleton, run, time) VALUES (1, 0, 0);
-- The corpus's stat index went (t33 K2): a corpus file's content is the observer's answer for its path.
DROP TABLE IF EXISTS corpus_stats;
-- The other memo plans a manifest names (`record_corpus_run_naming`): a prepared program's (decision 0073). They are
-- live while a kept manifest names them. Additive: an older binary ignores it, and collects nothing by it.
CREATE TABLE IF NOT EXISTS corpus_manifest_plans (
    manifest_id INTEGER NOT NULL,
    plan_digest BLOB NOT NULL CHECK(length(plan_digest) = 32),
    PRIMARY KEY (manifest_id, plan_digest)
) STRICT, WITHOUT ROWID;
-- The sweep of the corpus memo's retired schemas' rows went with `facts_results` (t33 step 3).
DROP TABLE IF EXISTS corpus_swept;

-- The single interner (atoms.rs, decisions 0049, 0401): dense, append-only; atom 0 is the empty string, and no
-- block's. The atoms are a chain of blocks in `<store>.atoms/`, mapped by every process: each row is a block, its
-- file's name there, its first id, the end the last commit reached in it, and the tag the block holds too (a block
-- not the row's is refused: a store copied without its folder). SQL reads atoms through the `atoms`
-- virtual table; a row that names an atom commits with or after the commit that reached it.
CREATE TABLE IF NOT EXISTS atom_blocks (
    seq INTEGER PRIMARY KEY CHECK(seq >= 1),
    name TEXT NOT NULL UNIQUE,
    first INTEGER NOT NULL UNIQUE CHECK(first >= 1),
    upto INTEGER NOT NULL CHECK(upto >= first),
    tag INTEGER NOT NULL
) STRICT;

-- A space's kept facts (decisions 0401, 0456 K4-K5): a chain of frozen files in `<store>.facts/<space>/`,
-- oldest first, each with its maker's tag (a file not its row's is refused). A run's write appends a
-- row; a merge replaces the newest rows with one. Metadata only: the rows are in the files.
CREATE TABLE IF NOT EXISTS facts_chain (
    space BLOB NOT NULL CHECK(length(space) = 32),
    seq INTEGER NOT NULL CHECK(seq >= 1),
    name TEXT NOT NULL,
    tag INTEGER NOT NULL,
    PRIMARY KEY(space, seq)
) STRICT;

-- The derivation graph (derivation.rs): fact names (atoms) bound to digests, and each
-- derivation's inputs with the digest it read. A derivation's output is a name too, so an
-- unchanged output rebinds nothing (early cutoff). Additive: older binaries never read these tables.
CREATE TABLE IF NOT EXISTS name_bindings (
    name_atom INTEGER PRIMARY KEY,
    digest BLOB NOT NULL CHECK(length(digest) = 32),
    revision INTEGER NOT NULL CHECK(revision >= 0)
) STRICT;
CREATE TABLE IF NOT EXISTS derivations (
    output_atom INTEGER PRIMARY KEY,
    producer BLOB NOT NULL CHECK(length(producer) = 32),
    status INTEGER NOT NULL,
    cost INTEGER NOT NULL CHECK(cost >= 0)
) STRICT;
CREATE TABLE IF NOT EXISTS derivation_inputs (
    output_atom INTEGER NOT NULL REFERENCES derivations(output_atom) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
    name_atom INTEGER,
    digest BLOB NOT NULL CHECK(length(digest) = 32),
    PRIMARY KEY (output_atom, ordinal)
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS derivation_inputs_by_name ON derivation_inputs(name_atom);
-- A push in progress (open-items O1): the row is committed before a push writes anything and
-- deleted after its last write reaches SQLite, so a store that holds it may hold a prefix of a
-- push's writes and is recovered before the next push.
CREATE TABLE IF NOT EXISTS push_pending (
    pending INTEGER PRIMARY KEY CHECK(pending = 1)
) STRICT;
-- The workspace pipeline's per-directory result pages and the predecessor's tree result segments went, never called
-- outside their tests (t33 step 3): a store made before loses their tables, and the binding index only they read.
DROP INDEX IF EXISTS name_bindings_by_digest;
DROP TABLE IF EXISTS tree_pages;
DROP TABLE IF EXISTS tree_pages_pending;
DROP TABLE IF EXISTS tree_result_segments;
DROP TABLE IF EXISTS completed_snapshot_trees;

-- The prefilter's per-tape access paths (prefilter.rs, from PIAT's anchor
-- store): kind positions, case-exact term postings and the nodes terms do not cover, each keyed by
-- tape and dying with it. Kinds are atoms (tape tag * 1024 + kind code). Postings are delta-varint
-- preorder lists (`nera_postings`). The frequency tables count over every tape that has a set.
CREATE TABLE IF NOT EXISTS prefilter_sets (
    tape_id INTEGER PRIMARY KEY REFERENCES tape_objects(tape_id) ON DELETE CASCADE,
    version INTEGER NOT NULL CHECK(version >= 0),
    nodes INTEGER NOT NULL CHECK(nodes >= 0),
    edges BLOB NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS tape_kind_counts (
    tape_id INTEGER NOT NULL REFERENCES prefilter_sets(tape_id) ON DELETE CASCADE,
    kind INTEGER NOT NULL,
    n INTEGER NOT NULL CHECK(n > 0),
    size INTEGER NOT NULL CHECK(size >= 0),
    postings BLOB NOT NULL,
    PRIMARY KEY (tape_id, kind)
) STRICT, WITHOUT ROWID;
-- Ingestion statistics (stats/ingest.rs, from Typing-support's `stats.py`): nodes of each kind
-- and the sum of their descendants, and direct edges by (parent kind, child kind, field; 0 is
-- none). A tape's own edges are its set's `edges` segment (`nera_edges`), subtracted when it goes.
CREATE TABLE IF NOT EXISTS kind_size (
    kind INTEGER PRIMARY KEY,
    n INTEGER NOT NULL,
    size INTEGER NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS edge_freq (
    pkind INTEGER NOT NULL,
    ckind INTEGER NOT NULL,
    field INTEGER NOT NULL,
    n INTEGER NOT NULL,
    PRIMARY KEY (pkind, ckind, field)
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS edge_freq_by_child ON edge_freq(ckind);
-- Fact relations' column statistics, merged per batch of documents (`ingest::merge_facts`):
-- per column, rows, distinct values, the sum of squared value counts corpus-wide (`sq`) and
-- within a document (`sqd`); per value of a value column, its count, documents and `sqd`.
CREATE TABLE IF NOT EXISTS fact_stats (
    tbl TEXT NOT NULL,
    col TEXT NOT NULL,
    rows INTEGER NOT NULL,
    ndv INTEGER NOT NULL,
    sq INTEGER NOT NULL,
    sqd INTEGER NOT NULL,
    PRIMARY KEY (tbl, col)
) STRICT, WITHOUT ROWID;
-- Compiled plans (stats/plans.rs), the statistics their decisions read and each plan's observed
-- selectivity and cost per backend are kept facts of the store (t33 step 6): a store made before
-- loses the tables.
DROP TABLE IF EXISTS compiled_plan_reads;
DROP TABLE IF EXISTS compiled_plans;
DROP TABLE IF EXISTS plan_feedback;
CREATE TABLE IF NOT EXISTS fact_value_stats (
    tbl TEXT NOT NULL,
    col TEXT NOT NULL,
    val ANY,
    n INTEGER NOT NULL,
    docs INTEGER NOT NULL,
    sqd INTEGER NOT NULL,
    PRIMARY KEY (tbl, col, val)
) STRICT, WITHOUT ROWID;
-- Every `term` column below is the term's atom (`atoms`): its text is stored once.
CREATE TABLE IF NOT EXISTS term_postings (
    lang INTEGER NOT NULL,
    term INTEGER NOT NULL,
    tape_id INTEGER NOT NULL REFERENCES prefilter_sets(tape_id) ON DELETE CASCADE,
    kind INTEGER NOT NULL,
    n INTEGER NOT NULL CHECK(n > 0),
    postings BLOB NOT NULL,
    PRIMARY KEY (lang, term, tape_id, kind)
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS term_postings_by_tape ON term_postings(tape_id);
CREATE TABLE IF NOT EXISTS term_freq (
    lang INTEGER NOT NULL,
    term INTEGER NOT NULL,
    n INTEGER NOT NULL,
    PRIMARY KEY (lang, term)
) STRICT, WITHOUT ROWID;
-- Planner statistics (stats/inputs.rs, from Data-layer-rule-expressivity's `value_frequency`
-- and `value_kind_frequency`): nodes per (term, kind) and covered nodes per kind, over every
-- tape with a set. A kind's uncovered nodes are `kind_size.n - term_kind_freq.n`.
CREATE TABLE IF NOT EXISTS term_node_freq (
    lang INTEGER NOT NULL,
    term INTEGER NOT NULL,
    kind INTEGER NOT NULL,
    n INTEGER NOT NULL,
    PRIMARY KEY (lang, term, kind)
) STRICT, WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS term_kind_freq (
    kind INTEGER PRIMARY KEY,
    n INTEGER NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS term_coverage (
    tape_id INTEGER NOT NULL REFERENCES prefilter_sets(tape_id) ON DELETE CASCADE,
    kind INTEGER NOT NULL,
    bad INTEGER NOT NULL,
    residue INTEGER NOT NULL,
    missing BLOB NOT NULL,
    PRIMARY KEY (tape_id, kind)
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS term_coverage_residue ON term_coverage(kind) WHERE residue > 0;
-- Publication adds the counts in one statement per table; removal, including a cascade from a
-- collected tape, counts down here.
CREATE TRIGGER IF NOT EXISTS tape_kind_count_delete AFTER DELETE ON tape_kind_counts BEGIN
    UPDATE kind_size SET n=n-OLD.n, size=size-OLD.size WHERE kind=OLD.kind;
    DELETE FROM kind_size WHERE kind=OLD.kind AND n<=0;
END;
CREATE TRIGGER IF NOT EXISTS prefilter_set_delete AFTER DELETE ON prefilter_sets BEGIN
    UPDATE edge_freq SET n=edge_freq.n-e.n FROM nera_edges(OLD.edges) e
        WHERE edge_freq.pkind=e.pkind AND edge_freq.ckind=e.ckind AND edge_freq.field=e.field;
    DELETE FROM edge_freq WHERE n<=0;
END;
CREATE TRIGGER IF NOT EXISTS term_posting_delete AFTER DELETE ON term_postings BEGIN
    UPDATE term_freq SET n=n-1 WHERE lang=OLD.lang AND term=OLD.term;
    DELETE FROM term_freq WHERE lang=OLD.lang AND term=OLD.term AND n<=0;
    UPDATE term_node_freq SET n=n-OLD.n WHERE lang=OLD.lang AND term=OLD.term AND kind=OLD.kind;
    DELETE FROM term_node_freq WHERE lang=OLD.lang AND term=OLD.term AND kind=OLD.kind AND n<=0;
    UPDATE term_kind_freq SET n=n-OLD.n WHERE kind=OLD.kind;
    DELETE FROM term_kind_freq WHERE kind=OLD.kind AND n<=0;
END;

-- The prefilter's regex vocabulary (vocabulary.rs, from PIAT's
-- `vocabulary.sql`): each registered predicate's matching terms, filled in SQL when a candidate
-- statement first names it and kept current by the terms each publication adds. A term that
-- leaves `term_freq` leaves every predicate's set by cascade.
CREATE TABLE IF NOT EXISTS vocabulary_predicate (
    lang INTEGER NOT NULL,
    expression TEXT NOT NULL,
    ready INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (lang, expression)
) STRICT, WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS vocabulary_term (
    lang INTEGER NOT NULL,
    expression TEXT NOT NULL,
    term INTEGER NOT NULL,
    PRIMARY KEY (lang, expression, term),
    FOREIGN KEY (lang, expression) REFERENCES vocabulary_predicate(lang, expression) ON DELETE CASCADE,
    FOREIGN KEY (lang, term) REFERENCES term_freq(lang, term) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS vocabulary_term_deleted ON vocabulary_term(lang, term);
-- An exact literal set (`^(?:a|b)$`) matches exactly its literals: a join, not a regex per term.
-- A literal is text: it need not be a term (or an atom) yet.
CREATE TABLE IF NOT EXISTS vocabulary_literal (
    lang INTEGER NOT NULL,
    expression TEXT NOT NULL,
    term TEXT NOT NULL,
    PRIMARY KEY (lang, term, expression),
    FOREIGN KEY (lang, expression) REFERENCES vocabulary_predicate(lang, expression) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS vocabulary_literal_expression ON vocabulary_literal(lang, expression);
CREATE TABLE IF NOT EXISTS vocabulary_pending (
    lang INTEGER NOT NULL,
    term INTEGER NOT NULL,
    PRIMARY KEY (lang, term),
    FOREIGN KEY (lang, term) REFERENCES term_freq(lang, term) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;
-- An upsert that only counts an existing term up is an UPDATE: only new terms are pending.
CREATE TRIGGER IF NOT EXISTS vocabulary_added AFTER INSERT ON term_freq BEGIN
    INSERT OR IGNORE INTO vocabulary_pending VALUES(NEW.lang, NEW.term);
END;
-- The text layer's vocabulary (code_rule_dsl/text, decision 0042): each kept plan's matching
-- terms by the plan's key. A key is filled over the whole vocabulary when first seen, then over
-- the terms added since: `text_pending` is a log of added terms in order, and `upto` the last
-- entry a key has read. Entries every key has read are dropped.
CREATE TABLE IF NOT EXISTS text_matcher (
    lang INTEGER NOT NULL,
    key TEXT NOT NULL,
    plan TEXT NOT NULL,
    upto INTEGER NOT NULL,
    PRIMARY KEY (lang, key)
) STRICT, WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS text_term (
    lang INTEGER NOT NULL,
    key TEXT NOT NULL,
    term INTEGER NOT NULL,
    PRIMARY KEY (lang, key, term),
    FOREIGN KEY (lang, key) REFERENCES text_matcher(lang, key) ON DELETE CASCADE,
    FOREIGN KEY (lang, term) REFERENCES term_freq(lang, term) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS text_term_deleted ON text_term(lang, term);
CREATE TABLE IF NOT EXISTS text_pending (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    lang INTEGER NOT NULL,
    term INTEGER NOT NULL
) STRICT;
CREATE TRIGGER IF NOT EXISTS text_vocabulary_added AFTER INSERT ON term_freq BEGIN
    INSERT INTO text_pending(lang, term) VALUES(NEW.lang, NEW.term);
END;
-- The per-file memo's entries (decision 0240 §4, 0295 §4): where each one's image is in the segments beside the
-- store (`<store>.memo/<pack>.seg`, `framed` 1: offset and length in the segment's block), by its plan's space and
-- its part (its content's and path's), with its path, its content's SHA-256 and its document key. A path's contents
-- are ordered by where they were written. `over_*`: what a later run kept over the entry, its identities (decision
-- 0280), in a segment too; `over_len` 0 is nothing. `framed` 0: an entry of the packs before (`<pack>.pack`,
-- compressed), not read (`ADDED_COLUMNS` adds these columns to an older store; an older binary reads a segment's row
-- as a pack's that is not there, a miss).
CREATE TABLE IF NOT EXISTS memo_entries(space BLOB NOT NULL, part INTEGER NOT NULL, path TEXT NOT NULL,
    digest BLOB NOT NULL, doc INTEGER NOT NULL, pack INTEGER NOT NULL, at INTEGER NOT NULL, len INTEGER NOT NULL,
    raw INTEGER NOT NULL, over_pack INTEGER NOT NULL DEFAULT 0, over_at INTEGER NOT NULL DEFAULT 0,
    over_len INTEGER NOT NULL DEFAULT 0, over_raw INTEGER NOT NULL DEFAULT 0, framed INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (space, part)) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS memo_entries_by_path ON memo_entries(space, path, pack, at);
-- The segments (and packs before) the entries are in.
CREATE TABLE IF NOT EXISTS memo_packs(pack INTEGER PRIMARY KEY);
-- Each guest's key (decision 0391): one row per identity (a SHA-256 of its range and parts), its key unique, its host
-- file (a value's first asker).
CREATE TABLE IF NOT EXISTS guest_keys(identity BLOB PRIMARY KEY, key INTEGER NOT NULL UNIQUE, path TEXT NOT NULL) WITHOUT ROWID;
"#;

impl ContentStore {
    /// Configures `connection` and brings its store up to SCHEMA. A store at this floor that holds
    /// every SCHEMA object is only read, so opening it never waits for another process's writer;
    /// creation, a floor raise or a missing object (an index this build added) takes the
    /// exclusive lock and applies SCHEMA. Returns the store's identity (`store_identity`).
    pub(super) fn initialize(connection: &mut Connection, flush: rerail_blocks::block::Flush) -> StoreResult<i64> {
        configure_connection(connection, flush)?;
        if compatible_version(connection)? != SCHEMA_VERSION || !holds_schema_objects(connection)? || holds_retired(connection)? {
            // A store made here is made by a writer's thread (decision 0104: a run's own threads only read): once,
            // before anything reads it.
            rerail_accel::exec::scoped(|scope| {
                scope
                    .spawn(|| -> StoreResult<()> {
                        super::hazards::behind_the_run();
                        let transaction =
                            connection.transaction_with_behavior(TransactionBehavior::Exclusive)?;
                        // Another open may have created the store since the read above.
                        let version = compatible_version(&transaction)?;
                        for (table, column, declaration) in missing_columns(&transaction)? {
                            transaction.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {declaration}"))?;
                            if (*table, *column) == ("analysis_scope_manifest", "recorded_run") {
                                transaction.execute_batch("UPDATE analysis_scope_manifest SET recorded_run=(
                                    SELECT last_run FROM corpus_manifests m WHERE m.manifest_id=analysis_scope_manifest.manifest_id)")?;
                            }
                        }
                        apply_schema(&transaction)?;
                        super::analysis_scope::compact_existing_directory_receipts(&transaction)?;
                        for table in RETIRED_TABLES {
                            transaction.execute_batch(&format!("DROP TABLE IF EXISTS {table}"))?;
                        }
                        if version != SCHEMA_VERSION {
                            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
                        }
                        transaction.commit()?;
                        Ok(())
                    })
                    .join()
                    .unwrap_or_else(|p| std::panic::resume_unwind(p))
            })?;
            if let Some(path) = connection.path() {
                super::atoms::forget_identity(path);
                super::held::forget_snapshot(path);
                // The retired tables' segments (`<store>.frames`): no row names them now.
                let mut frames = std::ffi::OsString::from(path);
                frames.push(".frames");
                let _ = std::fs::remove_dir_all(frames);
            }
        }
        let id: Option<i64> = connection
            .query_row_cached(
                "SELECT id FROM store_identity WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = id {
            return Ok(id);
        }
        // A copy of a store published without an identity (the tests' template, `corpus::test_store`): it becomes a
        // store of its own at its first open, as a store made here does, by a writer's thread.
        rerail_accel::exec::scoped(|scope| {
            scope
                .spawn(|| -> StoreResult<()> {
                    super::hazards::behind_the_run();
                    let transaction =
                        connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                    transaction.execute_cached(
                        "INSERT OR IGNORE INTO store_identity(singleton, id) VALUES (1, random())",
                        [],
                    )?;
                    transaction.commit()?;
                    Ok(())
                })
                .join()
                .unwrap_or_else(|p| std::panic::resume_unwind(p))
        })?;
        if let Some(path) = connection.path() {
            super::atoms::forget_identity(path);
        }
        Ok(connection.query_row_cached(
            "SELECT id FROM store_identity WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?)
    }
}

fn apply_schema(connection: &Connection) -> rusqlite::Result<()> {
    // The trigger also runs in connections from earlier binaries. Embed the same path
    // boundaries as analysis_scope::receipt_scopes using only SQLite's own functions.
    let ancestors = "WITH RECURSIVE boundaries(pos) AS (
        VALUES(0) UNION ALL
        SELECT pos+instr(substr(INPUT_SCOPE,pos+1),'PATH_SEP') FROM boundaries
        WHERE instr(substr(INPUT_SCOPE,pos+1),'PATH_SEP')>0
    ) SELECT INPUT_SCOPE UNION ALL SELECT substr(INPUT_SCOPE,1,pos-1) FROM boundaries WHERE pos>0
      UNION ALL SELECT substr(INPUT_SCOPE,1,pos) FROM boundaries WHERE pos>0"
        .replace("PATH_SEP", std::path::MAIN_SEPARATOR_STR);
    connection.execute_batch(&SCHEMA
        .replace("RR_PATH_SEPARATOR", std::path::MAIN_SEPARATOR_STR)
        .replace("RR_NEW_DIRECTORY_ANCESTORS", &ancestors.replace("INPUT_SCOPE", "NEW.scope"))
        .replace("RR_OLD_DIRECTORY_ANCESTORS", &ancestors.replace("INPUT_SCOPE", "OLD.scope")))
}

/// The store's floor, refused when it is newer than this build's.
fn compatible_version(connection: &Connection) -> StoreResult<i64> {
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(StoreError::Invalid(format!(
            "database schema version {version} is newer than supported version {SCHEMA_VERSION}: update Rerail"
        )));
    }
    // A store is never upgraded (STORE_FILE_NAME): an older floor's `atoms` table would hide the
    // `atoms` virtual table (decision 0049).
    if version != 0 && version < STORE_FILE_FLOOR {
        return Err(StoreError::Invalid(format!(
            "database schema version {version} is older than this build's floor {STORE_FILE_FLOOR}: the store is a cache; delete it"
        )));
    }
    Ok(version)
}

/// Whether the store holds every table, index, trigger and virtual table SCHEMA creates.
fn holds_schema_objects(connection: &Connection) -> StoreResult<bool> {
    let present = schema_object_names(connection)?
        .into_iter()
        .collect::<HashSet<_>>();
    let expected = SCHEMA_OBJECTS
        .as_ref()
        .map_err(|error| StoreError::Corrupt(format!("the schema does not apply: {error}")))?;
    Ok(expected.iter().all(|object| present.contains(object))
        && missing_columns(connection)?.is_empty())
}

/// Tables SCHEMA no longer makes, dropped from a store that has them when it is opened: their rows are no build's
/// to read. Dropping one raises no floor: an older binary makes it again (SCHEMA's `IF NOT EXISTS`), and this
/// build drops it again at its next open, a cache's rows lost.
///
/// - The frames (decision 0104's first form of the kept facts, and their bookkeeping of decisions 0117, 0138, 0227
///   and 0295 §4): the kept facts are blocks (decision 0439). A floor-7 store made where frames were still kept as
///   metadata beside the chain (decision 0456 step 6a) has them.
const RETIRED_TABLES: &[&str] = &["frames", "frame_segments", "frame_part_used", "frame_spaces", "frame_confirmed", "frame_used", "frame_program_used"];

/// Whether the store holds a table of `RETIRED_TABLES`.
fn holds_retired(connection: &Connection) -> StoreResult<bool> {
    let present: HashSet<(String, String)> = schema_object_names(connection)?.into_iter().collect();
    Ok(RETIRED_TABLES.iter().any(|t| present.contains(&("table".to_owned(), (*t).to_owned()))))
}

/// Columns SCHEMA's tables gained after stores were made with them, as `(table, column, declaration)`: `CREATE
/// TABLE IF NOT EXISTS` leaves an older store's table as it was, so applying SCHEMA adds these (`ALTER TABLE`).
/// Each has a default, so an older binary's inserts, which do not name it, still apply.
const ADDED_COLUMNS: &[(&str, &str, &str)] = &[
    ("store_state", "analysis_acquisitions", "INTEGER NOT NULL DEFAULT 0"),
    ("analysis_scope_workspace_snapshot", "acquisition", "INTEGER NOT NULL DEFAULT 0"),
    ("analysis_scope_snapshot", "acquired_at", "INTEGER NOT NULL DEFAULT 0"),
    ("analysis_scope_workspace_snapshot", "acquired_at", "INTEGER NOT NULL DEFAULT 0"),
    ("analysis_scope_manifest", "acquired_at", "INTEGER NOT NULL DEFAULT 0"),
    ("analysis_scope_manifest", "acquisition", "INTEGER NOT NULL DEFAULT 0"),
    ("analysis_scope_manifest", "recorded_run", "INTEGER NOT NULL DEFAULT 0"),
    ("analysis_scope_release", "scope", "TEXT NOT NULL DEFAULT ''"),
    ("analysis_scope_release", "before", "INTEGER NOT NULL DEFAULT 0"),
    ("analysis_scope_release", "legacy_scope", "TEXT"),
    ("analysis_scope_release", "legacy_acquisition", "INTEGER"),
    ("corpus_decided", "swept", "INTEGER NOT NULL DEFAULT 0"),
    ("corpus_state", "archive_due", "INTEGER NOT NULL DEFAULT 0"),
    ("corpus_manifests", "policy", "BLOB CHECK(policy IS NULL OR length(policy) = 32)"),
    ("memo_entries", "over_pack", "INTEGER NOT NULL DEFAULT 0"),
    ("memo_entries", "over_at", "INTEGER NOT NULL DEFAULT 0"),
    ("memo_entries", "over_len", "INTEGER NOT NULL DEFAULT 0"),
    ("memo_entries", "over_raw", "INTEGER NOT NULL DEFAULT 0"),
    ("memo_entries", "framed", "INTEGER NOT NULL DEFAULT 0"),
    ("source_objects", "seg", "INTEGER NOT NULL DEFAULT 0"),
    ("source_objects", "seg_at", "INTEGER NOT NULL DEFAULT 0"),
    ("source_objects", "seg_len", "INTEGER NOT NULL DEFAULT 0"),
    ("tape_objects", "seg", "INTEGER NOT NULL DEFAULT 0"),
    ("tape_objects", "seg_at", "INTEGER NOT NULL DEFAULT 0"),
    ("tape_objects", "seg_len", "INTEGER NOT NULL DEFAULT 0"),
    ("tape_objects", "programs", "INTEGER NOT NULL DEFAULT -1"),
    ("file_documents", "at_start", "INTEGER"),
    ("file_documents", "at_end", "INTEGER"),
];

/// The `ADDED_COLUMNS` a store's tables lack (of the tables it has).
fn missing_columns(
    connection: &Connection,
) -> StoreResult<Vec<&'static (&'static str, &'static str, &'static str)>> {
    let mut missing = Vec::new();
    for added in ADDED_COLUMNS {
        let (table, column, _) = added;
        let (tables, columns): (i64, i64) = connection.query_row(
            "SELECT (SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1), (SELECT count(*) FROM pragma_table_info(?1) WHERE name=?2)",
            params![table, column],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if tables > 0 && columns == 0 {
            missing.push(added);
        }
    }
    Ok(missing)
}

/// Every object SCHEMA creates, as `(type, name)`, read from its `CREATE … IF NOT EXISTS` statements. Its text is
/// the whole of it, so this cannot drift from SCHEMA (`schema_objects_are_what_schema_creates` applies SCHEMA and
/// compares). It was SCHEMA applied to an in-memory database, once a process: 124 statements written on every
/// open, which a warm run's check refuses (`RERAIL_WARM`). An object SQLite makes along with one of these (a
/// table's `sqlite_autoindex_*`, a full-text table's shadow tables) comes and goes with it.
static SCHEMA_OBJECTS: LazyLock<Result<Vec<(String, String)>, String>> =
    LazyLock::new(|| Ok(schema_objects(SCHEMA)));

fn schema_objects(schema: &str) -> Vec<(String, String)> {
    let text: String = schema
        .lines()
        .map(|l| l.split("--").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || c == '(' || c == ';')
        .filter(|w| !w.is_empty())
        .collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() {
        if words[i].eq_ignore_ascii_case("CREATE") {
            let rest = &words[i + 1..];
            let (kind, at) = match rest {
                [t, ..] if t.eq_ignore_ascii_case("TABLE") => ("table", 1),
                [v, t, ..]
                    if v.eq_ignore_ascii_case("VIRTUAL") && t.eq_ignore_ascii_case("TABLE") =>
                {
                    ("table", 2)
                }
                [u, t, ..]
                    if u.eq_ignore_ascii_case("UNIQUE") && t.eq_ignore_ascii_case("INDEX") =>
                {
                    ("index", 2)
                }
                [t, ..] if t.eq_ignore_ascii_case("INDEX") => ("index", 1),
                [t, ..] if t.eq_ignore_ascii_case("TRIGGER") => ("trigger", 1),
                [t, ..] if t.eq_ignore_ascii_case("VIEW") => ("view", 1),
                _ => {
                    i += 1;
                    continue;
                }
            };
            let rest = &rest[at..];
            let name = match rest {
                [a, b, c, name, ..]
                    if a.eq_ignore_ascii_case("IF")
                        && b.eq_ignore_ascii_case("NOT")
                        && c.eq_ignore_ascii_case("EXISTS") =>
                {
                    name
                }
                [name, ..] => name,
                [] => break,
            };
            out.push((kind.to_owned(), name.to_string()));
        }
        i += 1;
    }
    out
}

fn schema_object_names(connection: &Connection) -> StoreResult<Vec<(String, String)>> {
    connection
        .prepare("SELECT type,name FROM sqlite_schema")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::from)
}

#[cfg(test)]
mod schema_object_tests {
    use super::*;

    /// A store made before a column of `ADDED_COLUMNS` (here `corpus_decided.swept`, decision 0227) gains it when
    /// opened, its rows keeping their values.
    #[test]
    fn an_older_store_gains_the_added_columns() {
        let dir = std::env::temp_dir().join(format!("rerail-added-columns-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("store.sqlite");
        let _ = std::fs::remove_file(&path);
        drop(ContentStore::open(&path).unwrap());
        {
            let c = Connection::open(&path).unwrap();
            // The table as an older binary made it.
            c.execute_batch(
                "DROP TABLE corpus_decided; \
                 CREATE TABLE corpus_decided (singleton INTEGER PRIMARY KEY CHECK (singleton = 1), run INTEGER NOT NULL, time INTEGER NOT NULL) STRICT; \
                 INSERT INTO corpus_decided VALUES (1, 7, 0)",
            )
            .unwrap();
            assert_eq!(missing_columns(&c).unwrap().len(), 1);
        }
        drop(ContentStore::open(&path).unwrap());
        let c = Connection::open(&path).unwrap();
        assert!(missing_columns(&c).unwrap().is_empty());
        assert_eq!(
            c.query_row("SELECT run, swept FROM corpus_decided", [], |r| Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?
            )))
            .unwrap(),
            (7, 0)
        );
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A store made when the kept facts were frames loses their tables and segments when it is opened: no row of
    /// theirs is any build's to read (decision 0401 step 9, `RETIRED_TABLES`).
    #[test]
    fn a_store_of_before_loses_the_retired_tables_and_their_segments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.sqlite");
        drop(ContentStore::open(&path).unwrap());
        {
            let c = Connection::open(&path).unwrap();
            // As an older binary made them (named from the list, so no scan of the schema's tables reads them as ours).
            let (frames, spaces) = (RETIRED_TABLES[0], RETIRED_TABLES[3]);
            c.execute_batch(&format!("CREATE {} {frames}(space BLOB, data BLOB); INSERT INTO {frames} VALUES (x'01', x'02'); CREATE {} {spaces}(space BLOB PRIMARY KEY, runs INTEGER)", "TABLE", "TABLE")).unwrap();
            assert!(holds_retired(&c).unwrap());
        }
        std::fs::create_dir_all(dir.path().join("store.sqlite.frames")).unwrap();
        std::fs::write(dir.path().join("store.sqlite.frames/1.seg"), [0u8; 64]).unwrap();
        drop(ContentStore::open(&path).unwrap());
        let c = Connection::open(&path).unwrap();
        assert!(!holds_retired(&c).unwrap(), "dropped");
        assert!(!dir.path().join("store.sqlite.frames").exists(), "their segments removed");
        assert!(holds_schema_objects(&c).unwrap(), "the rest as it was");
    }

    /// The objects read from SCHEMA's text are those SCHEMA creates, less what SQLite makes along with them.
    #[test]
    fn schema_objects_are_what_schema_creates() {
        let connection = Connection::open_in_memory().unwrap();
        apply_schema(&connection).unwrap();
        let fts: Vec<String> = schema_objects(SCHEMA)
            .into_iter()
            .filter(|(_, n)| n.ends_with("_fts"))
            .map(|(_, n)| n)
            .collect();
        let mut applied: Vec<(String, String)> = schema_object_names(&connection)
            .unwrap()
            .into_iter()
            .filter(|(_, n)| {
                !n.starts_with("sqlite_") && !fts.iter().any(|f| n.starts_with(&format!("{f}_")))
            })
            .collect();
        let mut read = schema_objects(SCHEMA);
        applied.sort();
        read.sort();
        assert_eq!(read, applied);
        assert!(read.len() > 100, "{} objects", read.len());
    }
}
