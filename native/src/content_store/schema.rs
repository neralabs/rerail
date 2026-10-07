//! SQLite schema DDL and forward-only migrations.

use super::connection::configure_connection;
use super::{ContentStore, StoreError, StoreResult};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

// `user_version` is the compatibility floor: a binary refuses a store whose floor is newer than
// its SCHEMA_VERSION, so the floor moves only when an older binary would corrupt or misread the
// store. Additive changes that older SQL ignores, such as indexes, bump STORE_REVISION instead;
// every revision batch is idempotent and older binaries tolerate a newer revision.
//
// The managed installation's store is one file per machine (`~/.cache/rerail/`), shared by
// every installed version; a build running from a checkout keeps its own in `<checkout>/.rerail/`.
// Raising the floor therefore also renames the file (STORE_FILE_NAME), wherever it lives: older
// binaries keep their own store and a new store starts empty, since the store is a cache. The
// host joins the name (exported as `contentStoreFileName`) to its build's directory. The old file
// is neither migrated nor deleted: an older binary may still run against it, and no binary can
// tell that none will. It is safe to delete by hand once no older binary runs; the CHANGELOG
// entry of each floor raise names the file (with its -wal and -shm). STORE_FILE_FLOOR pins the
// pairing, so raising SCHEMA_VERSION alone does not compile.
pub(super) const SCHEMA_VERSION: i64 = 17;
pub(super) const STORE_REVISION: i64 = 18;

/// File name of every content store, managed or checkout-private, renamed whenever the floor rises.
pub const STORE_FILE_NAME: &str = "content-v3.sqlite";
/// The floor `STORE_FILE_NAME` was introduced at.
const STORE_FILE_FLOOR: i64 = 17;
const _: () = assert!(
    SCHEMA_VERSION == STORE_FILE_FLOOR,
    "raising the floor requires a new STORE_FILE_NAME"
);

pub(super) const SCHEMA_V1: &str = r#"
CREATE TABLE hashes (
    hash_id INTEGER PRIMARY KEY,
    algorithm_id INTEGER NOT NULL,
    lane0 INTEGER NOT NULL,
    lane1 INTEGER NOT NULL,
    lane2 INTEGER NOT NULL,
    lane3 INTEGER NOT NULL
) STRICT;
CREATE INDEX hashes_by_prefix ON hashes(algorithm_id, lane0);
CREATE TABLE strings (
    string_id INTEGER PRIMARY KEY,
    domain_id INTEGER NOT NULL,
    content_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    collision_ordinal INTEGER NOT NULL DEFAULT 0,
    value TEXT NOT NULL,
    UNIQUE (domain_id, content_hash_id, collision_ordinal)
) STRICT;
CREATE INDEX strings_by_hash ON strings(domain_id, content_hash_id);
CREATE TABLE paths (
    path_id INTEGER PRIMARY KEY,
    string_id INTEGER NOT NULL UNIQUE REFERENCES strings(string_id)
) STRICT;
CREATE TABLE source_objects (
    source_id INTEGER PRIMARY KEY,
    canonical_hash_id INTEGER NOT NULL UNIQUE REFERENCES hashes(hash_id),
    raw_length INTEGER NOT NULL,
    codec_id INTEGER NOT NULL,
    frame BLOB NOT NULL,
    created_generation INTEGER NOT NULL
) STRICT;
CREATE TABLE source_aliases (
    alias_hash_id INTEGER PRIMARY KEY REFERENCES hashes(hash_id),
    source_id INTEGER NOT NULL REFERENCES source_objects(source_id)
) STRICT;
CREATE INDEX source_aliases_by_source ON source_aliases(source_id);
CREATE TABLE tape_objects (
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
CREATE TABLE git_trees (
    tree_id INTEGER PRIMARY KEY,
    tree_hash_id INTEGER NOT NULL UNIQUE REFERENCES hashes(hash_id)
) STRICT;
CREATE TABLE git_tree_entries (
    tree_id INTEGER NOT NULL REFERENCES git_trees(tree_id),
    name_string_id INTEGER NOT NULL REFERENCES strings(string_id),
    mode_id INTEGER NOT NULL,
    object_kind_id INTEGER NOT NULL,
    target_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    PRIMARY KEY (tree_id, name_string_id)
) STRICT, WITHOUT ROWID;
CREATE TABLE snapshots (
    snapshot_id INTEGER PRIMARY KEY,
    root_tree_id INTEGER NOT NULL REFERENCES git_trees(tree_id),
    scanner_version INTEGER NOT NULL,
    grammar_set_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    tape_format_version INTEGER NOT NULL,
    UNIQUE (root_tree_id, scanner_version, grammar_set_hash_id, tape_format_version)
) STRICT;
CREATE TABLE snapshot_files (
    snapshot_id INTEGER NOT NULL REFERENCES snapshots(snapshot_id),
    path_id INTEGER NOT NULL REFERENCES paths(path_id),
    source_id INTEGER NOT NULL REFERENCES source_objects(source_id),
    tape_id INTEGER REFERENCES tape_objects(tape_id),
    PRIMARY KEY (snapshot_id, path_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX snapshot_files_by_source ON snapshot_files(source_id);
CREATE INDEX snapshot_files_by_tape ON snapshot_files(tape_id) WHERE tape_id IS NOT NULL;
CREATE TABLE workspaces (
    workspace_id INTEGER PRIMARY KEY,
    root_path_id INTEGER NOT NULL UNIQUE REFERENCES paths(path_id),
    repository_string_id INTEGER NOT NULL REFERENCES strings(string_id),
    owner_string_id INTEGER NOT NULL REFERENCES strings(string_id),
    snapshot_id INTEGER NOT NULL REFERENCES snapshots(snapshot_id),
    generation INTEGER NOT NULL,
    lease_until INTEGER NOT NULL,
    state_id INTEGER NOT NULL
) STRICT;
CREATE INDEX workspaces_by_lease ON workspaces(state_id, lease_until);
CREATE TABLE workspace_overlay (
    workspace_id INTEGER NOT NULL REFERENCES workspaces(workspace_id),
    path_id INTEGER NOT NULL REFERENCES paths(path_id),
    state_id INTEGER NOT NULL,
    source_id INTEGER REFERENCES source_objects(source_id),
    tape_id INTEGER REFERENCES tape_objects(tape_id),
    PRIMARY KEY (workspace_id, path_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX workspace_overlay_by_source ON workspace_overlay(source_id) WHERE source_id IS NOT NULL;
CREATE INDEX workspace_overlay_by_tape ON workspace_overlay(tape_id) WHERE tape_id IS NOT NULL;
CREATE TABLE object_pins (
    object_kind_id INTEGER NOT NULL,
    object_id INTEGER NOT NULL,
    owner_string_id INTEGER NOT NULL REFERENCES strings(string_id),
    lease_until INTEGER NOT NULL,
    PRIMARY KEY (object_kind_id, object_id, owner_string_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX object_pins_by_lease ON object_pins(lease_until);
CREATE TABLE store_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    generation INTEGER NOT NULL
) STRICT;
INSERT INTO store_state(singleton, generation) VALUES (1, 1);
"#;

const SCHEMA_V2: &str = r#"
CREATE TABLE candidate_catalogs (
    catalog_id INTEGER PRIMARY KEY,
    identity_hash_id INTEGER NOT NULL UNIQUE REFERENCES hashes(hash_id)
) STRICT;
CREATE TABLE candidate_checks (
    check_id INTEGER PRIMARY KEY,
    catalog_id INTEGER NOT NULL REFERENCES candidate_catalogs(catalog_id),
    policy_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    check_string_id INTEGER NOT NULL REFERENCES strings(string_id),
    UNIQUE (catalog_id, policy_hash_id, check_string_id)
) STRICT;
CREATE INDEX candidate_checks_by_catalog ON candidate_checks(catalog_id, check_id);
"#;

const SCHEMA_V3: &str = r#"
CREATE TABLE completed_workspace_revisions (
    workspace_id INTEGER PRIMARY KEY REFERENCES workspaces(workspace_id),
    revision INTEGER NOT NULL,
    catalog_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    completed_at INTEGER NOT NULL
) STRICT;
"#;

const SCHEMA_V4: &str = "";

const SCHEMA_V5: &str = r#"
CREATE TABLE local_check_results (
    source_id INTEGER NOT NULL REFERENCES source_objects(source_id),
    check_id INTEGER NOT NULL REFERENCES candidate_checks(check_id),
    codec_id INTEGER NOT NULL,
    raw_length INTEGER NOT NULL,
    frame BLOB NOT NULL,
    PRIMARY KEY(source_id, check_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX local_check_results_by_check ON local_check_results(check_id, source_id);
"#;

const SCHEMA_V6: &str = r#"
CREATE INDEX IF NOT EXISTS hashes_by_prefix ON hashes(algorithm_id, lane0);
"#;

const SCHEMA_V9: &str = r#"
CREATE TABLE completed_snapshot_trees (
    snapshot_id INTEGER NOT NULL REFERENCES snapshots(snapshot_id),
    tree_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    prefix TEXT NOT NULL,
    catalog_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    PRIMARY KEY (snapshot_id, tree_hash_id, prefix, catalog_hash_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX completed_snapshot_trees_by_identity
ON completed_snapshot_trees(tree_hash_id, prefix, catalog_hash_id);
"#;

const SCHEMA_V10: &str = r#"
CREATE TABLE tree_result_segments (
    tree_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    prefix TEXT NOT NULL,
    catalog_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    page_index INTEGER NOT NULL CHECK(page_index >= 0),
    page_count INTEGER NOT NULL CHECK(page_count > 0),
    raw_length INTEGER NOT NULL CHECK(raw_length >= 0),
    file_count INTEGER NOT NULL CHECK(file_count >= 0 AND file_count <= 64),
    frame BLOB NOT NULL,
    PRIMARY KEY (tree_hash_id, prefix, catalog_hash_id, page_index)
) STRICT, WITHOUT ROWID;
CREATE INDEX tree_result_segments_by_catalog
ON tree_result_segments(catalog_hash_id, prefix, tree_hash_id);
"#;

// Result pages were replaced by per-file results plus this directory-completion index.  A
// directory can be restored only when both its Git tree and the package-marker chain that
// determines Python module contexts agree.
const SCHEMA_V13: &str = r#"
DROP TABLE IF EXISTS tree_result_segments;
DROP TABLE IF EXISTS completed_snapshot_trees;
CREATE TABLE completed_directory_reuse (
    tree_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    prefix TEXT NOT NULL,
    package_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    catalog_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    PRIMARY KEY (tree_hash_id, prefix, package_hash_id, catalog_hash_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX completed_directory_reuse_by_catalog
ON completed_directory_reuse(catalog_hash_id, prefix, tree_hash_id, package_hash_id);
"#;

// Positional postings are compact blobs, not one SQL row per syntax node. A predicate is
// content-addressed from (index version, family, canonical key). Tape identity already includes
// source, grammar, grammar version, and tape format version, so ordinals cannot cross revisions.
const SCHEMA_V11: &str = r#"
CREATE TABLE anchor_index_sets (
    tape_id INTEGER NOT NULL REFERENCES tape_objects(tape_id) ON DELETE CASCADE,
    index_version INTEGER NOT NULL CHECK(index_version >= 0),
    node_count INTEGER NOT NULL CHECK(node_count >= 0),
    complete INTEGER NOT NULL CHECK(complete IN (0,1)),
    PRIMARY KEY(tape_id, index_version)
) STRICT, WITHOUT ROWID;
CREATE TABLE anchor_predicates (
    predicate_id INTEGER PRIMARY KEY,
    index_version INTEGER NOT NULL CHECK(index_version >= 0),
    family_id INTEGER NOT NULL CHECK(family_id > 0),
    canonical_hash_id INTEGER NOT NULL REFERENCES hashes(hash_id),
    key BLOB NOT NULL,
    UNIQUE(index_version, family_id, canonical_hash_id, key)
) STRICT;
CREATE INDEX anchor_predicates_by_hash
ON anchor_predicates(index_version, family_id, canonical_hash_id);
CREATE TABLE anchor_postings (
    tape_id INTEGER NOT NULL,
    index_version INTEGER NOT NULL,
    predicate_id INTEGER NOT NULL REFERENCES anchor_predicates(predicate_id),
    complete INTEGER NOT NULL CHECK(complete IN (0,1)),
    posting_count INTEGER NOT NULL CHECK(posting_count >= 0),
    postings BLOB NOT NULL,
    PRIMARY KEY(tape_id, index_version, predicate_id),
    FOREIGN KEY(tape_id, index_version)
      REFERENCES anchor_index_sets(tape_id, index_version) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;
CREATE INDEX anchor_postings_by_predicate
ON anchor_postings(predicate_id, tape_id, index_version);
"#;

// A query-independent search index. The kind stream contains one token per preorder node, so an
// FTS5 instance offset is the preorder ordinal. The exact-value and source-word streams select files;
// terminal transducers confirm their candidates. Contentless FTS stores no second copy of source.
const SCHEMA_V12: &str = r#"
DROP TABLE anchor_postings;
DROP TABLE anchor_predicates;
DROP TABLE anchor_index_sets;
CREATE TABLE search_index_sets (
    tape_id INTEGER PRIMARY KEY REFERENCES tape_objects(tape_id) ON DELETE CASCADE,
    index_version INTEGER NOT NULL CHECK(index_version >= 0),
    node_count INTEGER NOT NULL CHECK(node_count > 0),
    complete INTEGER NOT NULL CHECK(complete IN (0,1))
) STRICT;
CREATE VIRTUAL TABLE search_kind_fts USING fts5(
    token, content='', contentless_delete=1, detail=full, tokenize='ascii'
);
CREATE VIRTUAL TABLE search_value_fts USING fts5(
    token, content='', contentless_delete=1, detail=none, tokenize='ascii'
);
CREATE VIRTUAL TABLE search_text_fts USING fts5(
    token, content='', contentless_delete=1, detail=none,
    tokenize='unicode61 remove_diacritics 0'
);
CREATE VIRTUAL TABLE search_kind_instances USING fts5vocab(search_kind_fts, 'instance');
CREATE TRIGGER delete_tape_search_index BEFORE DELETE ON tape_objects BEGIN
    DELETE FROM search_kind_fts WHERE rowid=OLD.tape_id;
    DELETE FROM search_value_fts WHERE rowid=OLD.tape_id;
    DELETE FROM search_text_fts WHERE rowid=OLD.tape_id;
END;
"#;

// Selector indexes replaced persisted bit-lane facts and candidate manifests.
const SCHEMA_V14: &str = r#"
DROP TABLE IF EXISTS source_anchor_facts;
DROP TABLE IF EXISTS candidate_anchor_facts;
DROP TABLE IF EXISTS snapshot_candidate_files;
DROP TABLE IF EXISTS workspace_candidate_files;
DROP TABLE IF EXISTS snapshot_file_checks;
DROP TABLE IF EXISTS workspace_file_checks;
"#;

const SCHEMA_V15: &str = r#"
ALTER TABLE search_index_sets ADD COLUMN has_errors INTEGER NOT NULL DEFAULT 0 CHECK(has_errors IN (0,1));
DROP TABLE search_kind_instances;
-- Indexes built before this revision did not record syntax errors, and a tape with errors
-- must never be skipped undecoded: drop them; the maintenance thread rebuilds them.
DELETE FROM search_index_sets;
INSERT INTO search_kind_fts(search_kind_fts) VALUES('delete-all');
INSERT INTO search_value_fts(search_value_fts) VALUES('delete-all');
INSERT INTO search_text_fts(search_text_fts) VALUES('delete-all');
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
"#;

// Floor 17 excludes binaries that still write the exact-value and source-word FTS families, and
// collectors that deleted Git tree entries in batches. A tree is now complete only when its row
// says so; rows written before the marker existed are unproven until publication re-sends their
// entries. (Floor 16 only added indexes and is upgraded by the same step as floor 15.)
// Revision 18 adds the entry count that completeness is checked against.
const SCHEMA_V17: &str = r#"
DROP TRIGGER IF EXISTS delete_tape_search_index;
DROP TABLE IF EXISTS search_value_fts;
DROP TABLE IF EXISTS search_text_fts;
CREATE TRIGGER delete_tape_search_index BEFORE DELETE ON tape_objects BEGIN
    DELETE FROM search_kind_fts WHERE rowid=OLD.tape_id;
END;
ALTER TABLE git_trees ADD COLUMN complete INTEGER NOT NULL DEFAULT 0 CHECK(complete IN (0,1));
"#;

const STORE_REVISION_V13: &str = r#"
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
"#;

const STORE_REVISION_V15: &str = r#""#;

// Reverse-reference indexes for collection probes. The two dropped indexes repeated a primary key.
const STORE_REVISION_V17: &str = r#"
CREATE INDEX IF NOT EXISTS snapshot_files_by_path ON snapshot_files(path_id);
CREATE INDEX IF NOT EXISTS workspace_overlay_by_path ON workspace_overlay(path_id);
CREATE INDEX IF NOT EXISTS git_tree_entries_by_name ON git_tree_entries(name_string_id);
CREATE INDEX IF NOT EXISTS git_tree_entries_by_target ON git_tree_entries(target_hash_id);
CREATE INDEX IF NOT EXISTS context_check_results_by_context ON context_check_results(context_hash_id);
CREATE INDEX IF NOT EXISTS workspaces_by_repository ON workspaces(repository_string_id);
CREATE INDEX IF NOT EXISTS workspaces_by_owner ON workspaces(owner_string_id);
CREATE INDEX IF NOT EXISTS workspaces_by_snapshot ON workspaces(snapshot_id);
CREATE INDEX IF NOT EXISTS tape_objects_by_hash ON tape_objects(canonical_hash_id);
CREATE INDEX IF NOT EXISTS strings_by_content_hash ON strings(content_hash_id);
CREATE INDEX IF NOT EXISTS snapshots_by_grammar_set ON snapshots(grammar_set_hash_id);
CREATE INDEX IF NOT EXISTS candidate_checks_by_policy ON candidate_checks(policy_hash_id);
CREATE INDEX IF NOT EXISTS candidate_checks_by_string ON candidate_checks(check_string_id);
CREATE INDEX IF NOT EXISTS object_pins_by_owner ON object_pins(owner_string_id);
CREATE INDEX IF NOT EXISTS completed_workspace_revisions_by_catalog ON completed_workspace_revisions(catalog_hash_id);
CREATE INDEX IF NOT EXISTS completed_directory_reuse_by_package ON completed_directory_reuse(package_hash_id);
DROP INDEX IF EXISTS source_aliases_by_alias_hash;
DROP INDEX IF EXISTS completed_directory_reuse_by_tree;
"#;

// The entry count a tree's publication wrote. A tree is complete only while it still has that
// many entry rows, so a partial entry set is republished whichever writer left it; rows of
// binaries that do not write the count stay unproven. ADD COLUMN is not idempotent: guarded below.
const STORE_REVISION_V18: &str = "ALTER TABLE git_trees ADD COLUMN entry_count INTEGER;";

impl ContentStore {
    pub(super) fn initialize(connection: &mut Connection) -> StoreResult<()> {
        configure_connection(connection)?;
        Self::migrate(connection)
    }

    fn migrate(connection: &mut Connection) -> StoreResult<()> {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Exclusive)?;
        let version: i64 =
            transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(StoreError::Invalid(format!(
                "database schema version {version} is newer than supported version {SCHEMA_VERSION}"
            )));
        }
        let mut next = version;
        if next == 0 {
            transaction.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
            transaction.execute_batch(SCHEMA_V1)?;
            next = 1;
            transaction.pragma_update(None, "user_version", next)?;
        }
        if next == 1 {
            transaction.execute_batch(SCHEMA_V2)?;
            next = 2;
            transaction.pragma_update(None, "user_version", next)?;
        }
        if next == 2 {
            transaction.execute_batch(SCHEMA_V3)?;
            next = 3;
            transaction.pragma_update(None, "user_version", next)?;
        }
        if next == 3 {
            transaction.execute_batch(SCHEMA_V4)?;
            next = 4;
            transaction.pragma_update(None, "user_version", next)?;
        }
        if next == 4 {
            transaction.execute_batch(SCHEMA_V5)?;
            next = 5;
            transaction.pragma_update(None, "user_version", next)?;
        }
        if next == 5 {
            transaction.execute_batch(SCHEMA_V6)?;
            next = 6;
            transaction.pragma_update(None, "user_version", next)?;
        }
        if next != 6
            && next != 7
            && next != 8
            && next != 9
            && next != 10
            && next != 11
            && next != 12
            && next != 13
            && next != 14
            && next != 15
            && next != 16
            && next != SCHEMA_VERSION
        {
            return Err(StoreError::Invalid(format!(
                "database schema version {next} has no migration to {SCHEMA_VERSION}"
            )));
        }
        transaction.commit()?;
        if next == 6 {
            Self::migrate_hash_prefix_index(connection)?;
            next = 7;
        }
        if next == 7 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Exclusive)?;
            next = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
            if next == 7 {
                transaction.execute_batch(
                    "DROP TABLE IF EXISTS maintenance_lease; PRAGMA user_version=8;",
                )?;
                next = 8;
            }
            transaction.commit()?;
        }
        if next == 8 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Exclusive)?;
            next = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
            if next == 8 {
                transaction.execute_batch(SCHEMA_V9)?;
                transaction.pragma_update(None, "user_version", 9)?;
                next = 9;
            }
            transaction.commit()?;
        }
        if next == 9 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Exclusive)?;
            next = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
            if next == 9 {
                transaction.execute_batch(SCHEMA_V10)?;
                transaction.pragma_update(None, "user_version", 10)?;
                next = 10;
            }
            transaction.commit()?;
        }
        if next == 10 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Exclusive)?;
            next = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
            if next == 10 {
                transaction.execute_batch(SCHEMA_V11)?;
                transaction.pragma_update(None, "user_version", 11)?;
                next = 11;
            }
            transaction.commit()?;
        }
        if next == 11 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Exclusive)?;
            next = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
            if next == 11 {
                transaction.execute_batch(SCHEMA_V12)?;
                transaction.pragma_update(None, "user_version", 12)?;
                next = 12;
            }
            transaction.commit()?;
        }
        if next == 12 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Exclusive)?;
            next = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
            if next == 12 {
                transaction.execute_batch(SCHEMA_V13)?;
                transaction.pragma_update(None, "user_version", 13)?;
                next = 13;
            }
            transaction.commit()?;
        }
        if next == 13 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Exclusive)?;
            next = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
            if next == 13 {
                transaction.execute_batch(SCHEMA_V14)?;
                transaction.pragma_update(None, "user_version", 14)?;
                next = 14;
            }
            transaction.commit()?;
        }
        if next == 14 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Exclusive)?;
            next = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
            if next == 14 {
                transaction.execute_batch(SCHEMA_V15)?;
                transaction.pragma_update(None, "user_version", 15)?;
                next = 15;
            }
            transaction.commit()?;
        }
        if next == 15 || next == 16 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Exclusive)?;
            next = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
            if next == 15 || next == 16 {
                transaction.execute_batch(SCHEMA_V17)?;
                transaction.pragma_update(None, "user_version", 17)?;
            }
            transaction.commit()?;
        }
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Exclusive)?;
        transaction.execute_batch(
            "CREATE TABLE IF NOT EXISTS store_schema_revision (
                singleton INTEGER PRIMARY KEY CHECK(singleton=1),
                revision INTEGER NOT NULL
            ) STRICT;",
        )?;
        // A store without a recorded revision has run no revision batch yet.
        transaction.execute(
            "INSERT OR IGNORE INTO store_schema_revision(singleton,revision) VALUES(1,0)",
            [],
        )?;
        let revision: i64 = transaction.query_row(
            "SELECT revision FROM store_schema_revision WHERE singleton=1",
            [],
            |row| row.get(0),
        )?;
        if revision < STORE_REVISION {
            transaction.execute_batch(STORE_REVISION_V13)?;
            transaction.execute_batch(STORE_REVISION_V15)?;
            transaction.execute_batch(STORE_REVISION_V17)?;
            let has_entry_count: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('git_trees') WHERE name='entry_count')",
                [],
                |row| row.get(0),
            )?;
            if !has_entry_count {
                transaction.execute_batch(STORE_REVISION_V18)?;
            }
            transaction.execute(
                "UPDATE store_schema_revision SET revision=?1 WHERE singleton=1",
                [STORE_REVISION],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    fn migrate_hash_prefix_index(connection: &mut Connection) -> StoreResult<()> {
        connection.pragma_update(None, "foreign_keys", "OFF")?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Exclusive)?;
        let version: i64 =
            transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version >= 7 {
            transaction.commit()?;
            connection.pragma_update(None, "foreign_keys", "ON")?;
            return Ok(());
        }
        transaction.execute_batch(
            "DROP INDEX IF EXISTS hashes_by_prefix;
             CREATE TABLE hashes_v7(
               hash_id INTEGER PRIMARY KEY,
               algorithm_id INTEGER NOT NULL,
               lane0 INTEGER NOT NULL,
               lane1 INTEGER NOT NULL,
               lane2 INTEGER NOT NULL,
               lane3 INTEGER NOT NULL
             ) STRICT;
             INSERT INTO hashes_v7(hash_id,algorithm_id,lane0,lane1,lane2,lane3)
               SELECT hash_id,algorithm_id,lane0,lane1,lane2,lane3 FROM hashes;
             DROP TABLE hashes;
             ALTER TABLE hashes_v7 RENAME TO hashes;
             CREATE INDEX hashes_by_prefix ON hashes(algorithm_id,lane0);
             PRAGMA user_version=7;",
        )?;
        transaction.commit()?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let violation = connection
            .query_row("PRAGMA foreign_key_check", [], |_| Ok(()))
            .optional()?;
        if violation.is_some() {
            return Err(StoreError::Corrupt(
                "hash-prefix migration broke a foreign key".to_owned(),
            ));
        }
        Ok(())
    }
}
