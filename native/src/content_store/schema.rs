//! The content store's SQLite schema: one current, idempotent DDL.

use super::connection::configure_connection;
use super::{ContentStore, StoreError, StoreResult};
use rusqlite::{Connection, TransactionBehavior};

// `user_version` is the compatibility floor: a binary refuses a store whose floor is newer than
// its SCHEMA_VERSION, so the floor moves only when an older binary would corrupt or misread the
// store. Every open applies SCHEMA, which is idempotent, so additive changes that older SQL
// ignores, such as indexes, need no floor raise: older binaries tolerate them.
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
pub(super) const SCHEMA_VERSION: i64 = 1;

/// File name of every content store, managed or checkout-private, renamed whenever the floor rises.
pub const STORE_FILE_NAME: &str = "content-v1.sqlite";
/// The floor `STORE_FILE_NAME` was introduced at.
const STORE_FILE_FLOOR: i64 = 1;
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
    created_generation INTEGER NOT NULL
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
    UNIQUE (source_id, grammar_id, grammar_version, tape_format_version)
) STRICT;
CREATE INDEX IF NOT EXISTS tape_objects_by_hash ON tape_objects(canonical_hash_id);

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
    generation INTEGER NOT NULL
) STRICT;
INSERT OR IGNORE INTO store_state(singleton, generation) VALUES (1, 1);

CREATE TABLE IF NOT EXISTS candidate_catalogs (
    catalog_id INTEGER PRIMARY KEY,
    identity_hash_id INTEGER NOT NULL UNIQUE REFERENCES hashes(hash_id)
) STRICT;
CREATE TABLE IF NOT EXISTS candidate_checks (
    check_id INTEGER PRIMARY KEY,
    catalog_id INTEGER NOT NULL REFERENCES candidate_catalogs(catalog_id),
    policy_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    check_string_id INTEGER NOT NULL REFERENCES strings(string_id),
    UNIQUE (catalog_id, policy_hash_id, check_string_id)
) STRICT;
CREATE INDEX IF NOT EXISTS candidate_checks_by_catalog ON candidate_checks(catalog_id, check_id);
CREATE INDEX IF NOT EXISTS candidate_checks_by_policy ON candidate_checks(policy_hash_id);
CREATE INDEX IF NOT EXISTS candidate_checks_by_string ON candidate_checks(check_string_id);
CREATE TABLE IF NOT EXISTS completed_workspace_revisions (
    workspace_id INTEGER PRIMARY KEY REFERENCES workspaces(workspace_id),
    revision INTEGER NOT NULL,
    catalog_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    completed_at INTEGER NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS completed_workspace_revisions_by_catalog
ON completed_workspace_revisions(catalog_hash_id);
CREATE TABLE IF NOT EXISTS local_check_results (
    source_id INTEGER NOT NULL REFERENCES source_objects(source_id),
    check_id INTEGER NOT NULL REFERENCES candidate_checks(check_id),
    codec_id INTEGER NOT NULL,
    raw_length INTEGER NOT NULL,
    frame BLOB NOT NULL,
    PRIMARY KEY(source_id, check_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS local_check_results_by_check ON local_check_results(check_id, source_id);
CREATE TABLE IF NOT EXISTS context_check_results (
    source_id INTEGER NOT NULL REFERENCES source_objects(source_id) ON DELETE CASCADE,
    context_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id) ON DELETE CASCADE,
    check_id INTEGER NOT NULL REFERENCES candidate_checks(check_id) ON DELETE CASCADE,
    codec_id INTEGER NOT NULL,
    raw_length INTEGER NOT NULL,
    frame BLOB NOT NULL,
    PRIMARY KEY(source_id, context_hash_id, check_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS context_check_results_by_check
ON context_check_results(check_id, source_id);
CREATE INDEX IF NOT EXISTS context_check_results_by_context
ON context_check_results(context_hash_id);

-- A directory can be restored only when both its Git tree and the package-marker chain that
-- determines Python module contexts agree.
CREATE TABLE IF NOT EXISTS completed_directory_reuse (
    tree_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    prefix TEXT NOT NULL,
    package_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    catalog_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    PRIMARY KEY (tree_hash_id, prefix, package_hash_id, catalog_hash_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS completed_directory_reuse_by_catalog
ON completed_directory_reuse(catalog_hash_id, prefix, tree_hash_id, package_hash_id);
CREATE INDEX IF NOT EXISTS completed_directory_reuse_by_package
ON completed_directory_reuse(package_hash_id);

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
CREATE TRIGGER IF NOT EXISTS delete_tape_search_index BEFORE DELETE ON tape_objects BEGIN
    DELETE FROM search_kind_fts WHERE rowid=OLD.tape_id;
END;
"#;

impl ContentStore {
    pub(super) fn initialize(connection: &mut Connection) -> StoreResult<()> {
        configure_connection(connection)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Exclusive)?;
        let version: i64 =
            transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(StoreError::Invalid(format!(
                "database schema version {version} is newer than supported version {SCHEMA_VERSION}: update Rerail"
            )));
        }
        if version == 0 {
            transaction.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
        }
        transaction.execute_batch(SCHEMA)?;
        if version != SCHEMA_VERSION {
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        transaction.commit()?;
        Ok(())
    }
}
