//! The content store's SQLite schema: one current, idempotent DDL.

use super::connection::configure_connection;
use super::{ContentStore, StoreError, StoreResult};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
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
pub(super) const SCHEMA_VERSION: i64 = 4;

/// File name of every content store, managed or checkout-private, renamed whenever the floor rises.
pub const STORE_FILE_NAME: &str = "content-v4.sqlite";
/// The floor `STORE_FILE_NAME` was introduced at. 4: a corpus manifest's lane is a column of it
/// (`corpus_manifests.lane`), not a table of its own (decision 0104); 3 was atoms stored as segments
/// of the arena (decision 0049); 2 the prefilter vocabulary keyed by atom.
const STORE_FILE_FLOOR: i64 = 4;
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

-- Which store this file is: random when it is created. What a process holds of a store by its path (its atoms,
-- atoms.rs; its threads' connections, connection.rs) belongs to the store of this identity only, so a store
-- deleted and created again at the same path, by any process, is not read with the old one's. Additive: older
-- binaries never read it.
CREATE TABLE IF NOT EXISTS store_identity (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    id INTEGER NOT NULL
) STRICT;
INSERT OR IGNORE INTO store_identity(singleton, id) VALUES (1, random());

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
-- Physical rule planning reads exact term frequencies from SQL instead of estimating them in Rust.
CREATE VIRTUAL TABLE IF NOT EXISTS search_kind_vocab USING fts5vocab(search_kind_fts, 'row');
CREATE TRIGGER IF NOT EXISTS delete_tape_search_index BEFORE DELETE ON tape_objects BEGIN
    DELETE FROM search_kind_fts WHERE rowid=OLD.tape_id;
END;

-- The facts engine's memo (facts_memo.rs): per-file clause results keyed by content (a source),
-- fact schema version, plan digest and, for a clause that reads the path, the path ('' for a
-- content-pure one). Additive: every row cascades from its source and names no other shared row
-- (hashes, strings, paths), so an older binary's collection can delete sources under it without
-- orphaning or misreading a row, and the floor stays.
CREATE TABLE IF NOT EXISTS facts_plan_sets (
    set_digest BLOB NOT NULL CHECK(length(set_digest) = 32),
    ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
    plan_digest BLOB NOT NULL CHECK(length(plan_digest) = 32),
    PRIMARY KEY (set_digest, ordinal)
) STRICT, WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS facts_results (
    source_id INTEGER NOT NULL REFERENCES source_objects(source_id) ON DELETE CASCADE,
    fact_schema INTEGER NOT NULL CHECK(fact_schema >= 0),
    path TEXT NOT NULL,
    plan_digest BLOB NOT NULL CHECK(length(plan_digest) = 32),
    codec_id INTEGER NOT NULL,
    raw_length INTEGER NOT NULL,
    frame BLOB NOT NULL,
    PRIMARY KEY (source_id, fact_schema, path, plan_digest)
) STRICT, WITHOUT ROWID;
-- Neural Engine models compiled on this machine (decision 0098 §3): a compiled model's archive under the digest of
-- its template and its fixed relation. A JIT-compiled model is a fact of the store, read back by later runs; an AOT
-- one is part of the executable. Additive and referencing nothing, so the floor stays.
CREATE TABLE IF NOT EXISTS accel_models (
    model_key BLOB NOT NULL PRIMARY KEY CHECK(length(model_key) = 32),
    archive BLOB NOT NULL
) STRICT, WITHOUT ROWID;
-- Bit i of `words` (little-endian u64s): the set's clause i ran with nothing to report; its
-- counts are in the `counts` frame. A clear bit is unknown, never absent.
CREATE TABLE IF NOT EXISTS facts_completions (
    source_id INTEGER NOT NULL REFERENCES source_objects(source_id) ON DELETE CASCADE,
    fact_schema INTEGER NOT NULL CHECK(fact_schema >= 0),
    path TEXT NOT NULL,
    set_digest BLOB NOT NULL CHECK(length(set_digest) = 32),
    words BLOB NOT NULL,
    codec_id INTEGER NOT NULL,
    raw_length INTEGER NOT NULL,
    counts BLOB NOT NULL,
    PRIMARY KEY (source_id, fact_schema, path, set_digest)
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS facts_completions_by_set ON facts_completions(set_digest);

-- The cross-file memo's stored rows (decision 0076, cross_rows.rs). `cross_components` maps a component's digest
-- (its plan, key and documents' keys) to a source holding that descriptor; `cross_rows` holds its rows of each
-- memoised relation, clustered by (source, relation), so a run's lookup and its replay are one query each. Additive:
-- every row cascades from its source, which a run's manifest names (decision 0053).
CREATE TABLE IF NOT EXISTS cross_components (
    digest TEXT PRIMARY KEY,
    source_id INTEGER NOT NULL REFERENCES source_objects(source_id) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS cross_components_by_source ON cross_components(source_id);
CREATE TABLE IF NOT EXISTS cross_rows (
    source_id INTEGER NOT NULL REFERENCES source_objects(source_id) ON DELETE CASCADE,
    rel TEXT NOT NULL,
    seq INTEGER NOT NULL,
    c0 INTEGER, c1 INTEGER, c2 INTEGER, c3 INTEGER, c4 INTEGER, c5 INTEGER, c6 INTEGER, c7 INTEGER,
    PRIMARY KEY (source_id, rel, seq)
) STRICT, WITHOUT ROWID;
-- The rows as blocks (cross_rows.rs): a component's rows of one relation, its columns delta-coded, one row each.
-- `cross_rows` is what older binaries write and read; components stored as blocks are named apart from theirs.
CREATE TABLE IF NOT EXISTS cross_blocks (
    source_id INTEGER NOT NULL REFERENCES source_objects(source_id) ON DELETE CASCADE,
    rel TEXT NOT NULL,
    width INTEGER NOT NULL,
    rows INTEGER NOT NULL,
    data BLOB NOT NULL,
    PRIMARY KEY (source_id, rel)
) STRICT, WITHOUT ROWID;

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
    UNIQUE (root_id, digest)
) STRICT;
CREATE INDEX IF NOT EXISTS corpus_manifests_by_run ON corpus_manifests(last_run);
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
-- [name, type] (docs/sql-access/01-design.md §2): a session reads a run's frames by name. Collected with the plan.
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
    due INTEGER NOT NULL
) STRICT;
INSERT OR IGNORE INTO corpus_state(singleton, run, history_bytes, due) VALUES (1, 0, 0, 0);
-- The last keep decision: the run counter and unix second it was made at. A new manifest makes a decision due, but
-- a pass makes one only when it can change something (`CorpusPolicy`): a root past its protected runs since, or
-- enough runs or seconds since. A new root's first runs are protected, so a run of fresh roots decides rarely.
-- Additive: an older binary ignores it.
CREATE TABLE IF NOT EXISTS corpus_decided (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    run INTEGER NOT NULL,
    time INTEGER NOT NULL
) STRICT;
INSERT OR IGNORE INTO corpus_decided(singleton, run, time) VALUES (1, 0, 0);
-- A root's stat index (corpus_runs.rs), as git's: a file's size, mtime and inode when its source was last read, and
-- the source. A run trusts an entry whose stat matches and whose mtime is over two seconds before `recorded_ns` (an
-- edit in the same tick as the recording could otherwise keep its stat), and then neither reads nor hashes the file.
-- An entry whose source was collected is not trusted. Additive: an older binary ignores it.
CREATE TABLE IF NOT EXISTS corpus_stats (
    root_id INTEGER NOT NULL REFERENCES corpus_roots(root_id),
    path TEXT NOT NULL,
    size INTEGER NOT NULL,
    mtime_ns INTEGER NOT NULL,
    inode INTEGER NOT NULL,
    source_id INTEGER NOT NULL,
    recorded_ns INTEGER NOT NULL,
    PRIMARY KEY (root_id, path)
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS facts_results_by_plan ON facts_results(plan_digest);
-- The other memo plans a manifest names (`record_corpus_run_naming`): a prepared program's (decision 0073). They are
-- live while a kept manifest names them. Additive: an older binary ignores it, and collects nothing by it.
CREATE TABLE IF NOT EXISTS corpus_manifest_plans (
    manifest_id INTEGER NOT NULL,
    plan_digest BLOB NOT NULL CHECK(length(plan_digest) = 32),
    PRIMARY KEY (manifest_id, plan_digest)
) STRICT, WITHOUT ROWID;
-- The sweep of the corpus memo's retired schemas (`RETIRED_CORPUS_MEMO_SCHEMAS`): done once a pass finds no more.
CREATE TABLE IF NOT EXISTS corpus_swept (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    done INTEGER NOT NULL
) STRICT;
INSERT OR IGNORE INTO corpus_swept(singleton, done) VALUES (1, 0);

-- The single interner (atoms.rs, decision 0049): dense, append-only; atom 0 is the empty string.
-- Each row is a run of consecutive atoms from `first`: their end offsets (u32, little-endian,
-- relative to the row's bytes) and their bytes. SQL reads atoms through the `atoms` virtual table
-- over the process's arena; a row that names an atom commits with or after its segment.
CREATE TABLE IF NOT EXISTS atom_segment (
    first INTEGER PRIMARY KEY CHECK(first >= 0),
    ends BLOB NOT NULL,
    bytes BLOB NOT NULL
) STRICT;
INSERT OR IGNORE INTO atom_segment(first,ends,bytes) VALUES (0,x'00000000',x'');

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
-- Collection keeps pages some binding names (tree_pages.rs).
CREATE INDEX IF NOT EXISTS name_bindings_by_digest ON name_bindings(digest);

-- Per-directory result pages (tree_pages.rs, from Tempo's tree_result_segments), keyed by what the
-- directory's result depends on; `digest` is the page derivation's output digest.
CREATE TABLE IF NOT EXISTS tree_pages (
    page_key BLOB NOT NULL CHECK(length(page_key) = 32),
    page_index INTEGER NOT NULL CHECK(page_index >= 0),
    page_count INTEGER NOT NULL CHECK(page_count > 0),
    digest BLOB NOT NULL CHECK(length(digest) = 32),
    codec_id INTEGER NOT NULL,
    raw_length INTEGER NOT NULL CHECK(raw_length >= 0),
    file_count INTEGER NOT NULL CHECK(file_count >= 0 AND file_count <= 64),
    frame BLOB NOT NULL,
    PRIMARY KEY (page_key, page_index)
) STRICT, WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS tree_pages_by_digest ON tree_pages(digest);

-- The prefilter's per-tape access paths (prefilter.rs, from Pulling-it-all-together's anchor
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
-- Compiled plans (stats/plans.rs): decisions and programs under the plan's identity, each
-- statistic a decision read with the interval over which the decision stays valid, and the
-- observed selectivity and cost per root of each plan per backend.
CREATE TABLE IF NOT EXISTS compiled_plans (
    plan BLOB PRIMARY KEY CHECK(length(plan) = 32),
    compiler INTEGER NOT NULL,
    decisions TEXT NOT NULL,
    program BLOB NOT NULL
) STRICT, WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS compiled_plan_reads (
    plan BLOB NOT NULL REFERENCES compiled_plans(plan) ON DELETE CASCADE,
    stat TEXT NOT NULL,
    lo INTEGER NOT NULL,
    hi INTEGER NOT NULL,
    PRIMARY KEY (plan, stat)
) STRICT, WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS plan_feedback (
    plan BLOB NOT NULL,
    backend TEXT NOT NULL,
    selectivity REAL NOT NULL,
    ns_per_root REAL NOT NULL,
    samples INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (plan, backend)
) STRICT, WITHOUT ROWID;
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

-- The prefilter's regex vocabulary (vocabulary.rs, from Pulling-it-all-together's
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
-- A run's relations as stored frames (decision 0104, `frames`), by space, relation, identity and part.
CREATE TABLE IF NOT EXISTS frames(space BLOB NOT NULL, rel TEXT NOT NULL, ident BLOB NOT NULL,
    part INTEGER NOT NULL, digest INTEGER NOT NULL, reads BLOB NOT NULL, width INTEGER NOT NULL, rows INTEGER NOT NULL,
    types BLOB NOT NULL, data BLOB NOT NULL, used INTEGER NOT NULL, PRIMARY KEY (space, rel, ident, part)) WITHOUT ROWID;
-- A memo frame's digest is its document's key, a function of its path (`memo::doc_keys`): a path's contents are found
-- by it, without reading every frame of the relation (each a page, its rows inline).
CREATE INDEX IF NOT EXISTS frames_by_digest ON frames(space, rel, digest);
-- The run each frame was last used by (decision 0138): apart from `frames`, whose records hold the frame's encoded
-- rows, since SQLite rewrites a whole record (its overflow pages too) to change one column, and reads past `data`
-- to read `used`. Every frame has its row here: put with it, forgotten with it. `frames.used` is the run that put
-- the frame, for an older binary. A store made before this table gets its rows from `frames.used`, once: SCHEMA is
-- applied when an object is missing, and the rows are then all there (`OR IGNORE`).
CREATE TABLE IF NOT EXISTS frame_part_used(space BLOB NOT NULL, rel TEXT NOT NULL, ident BLOB NOT NULL,
    part INTEGER NOT NULL, used INTEGER NOT NULL, PRIMARY KEY (space, rel, ident, part)) WITHOUT ROWID;
INSERT OR IGNORE INTO frame_part_used(space, rel, ident, part, used) SELECT space, rel, ident, part, used FROM frames;
-- How many runs each space has counted.
CREATE TABLE IF NOT EXISTS frame_spaces(space BLOB PRIMARY KEY, runs INTEGER NOT NULL) WITHOUT ROWID;
-- The spaces' confirmations (decision 0117 step 5): each frame found unchanged under other reads than it names.
CREATE TABLE IF NOT EXISTS frame_confirmed(space BLOB PRIMARY KEY, data BLOB NOT NULL) WITHOUT ROWID;
-- Each run's use of a space (decision 0117 step 5): the relations its programs used, as sorted keys, one row a
-- run, the last `frames::RUNS` runs'. A frame is collected once the run that put it is that old and no run since
-- used its relation.
CREATE TABLE IF NOT EXISTS frame_used(space BLOB NOT NULL, run INTEGER NOT NULL, rels BLOB NOT NULL,
    PRIMARY KEY (space, run)) WITHOUT ROWID;
-- Each run's use of a space by program (decision 0117 step 5, O-store-single-program): the relations it used, as
-- sorted keys, one row a program a run, each program's last `frames::RUNS`. Programs over one root share its space;
-- counted by the space's runs alone, one policy's frames went unused while the others ran, and were collected. A
-- frame is collected once the run that put it is that old and none of these rows names its relation. `frame_used`
-- (one row a run, of every program) is read no more: it stays for an older binary.
CREATE TABLE IF NOT EXISTS frame_program_used(space BLOB NOT NULL, program BLOB NOT NULL, run INTEGER NOT NULL,
    rels BLOB NOT NULL, PRIMARY KEY (space, program, run)) WITHOUT ROWID;
"#;

impl ContentStore {
    /// Configures `connection` and brings its store up to SCHEMA. A store at this floor that holds
    /// every SCHEMA object is only read, so opening it never waits for another process's writer;
    /// creation, a floor raise or a missing object (an index this build added) takes the
    /// exclusive lock and applies SCHEMA. Returns the store's identity (`store_identity`).
    pub(super) fn initialize(connection: &mut Connection) -> StoreResult<i64> {
        configure_connection(connection)?;
        if compatible_version(connection)? != SCHEMA_VERSION || !holds_schema_objects(connection)? {
            // A store made here is made by a writer's thread (decision 0104: a run's own threads only read): once,
            // before anything reads it.
            rerail_accel::exec::scoped(|scope| {
                scope
                    .spawn(|| -> StoreResult<()> {
                        super::hazards::behind_the_run();
                        let transaction = connection.transaction_with_behavior(TransactionBehavior::Exclusive)?;
                        // Another open may have created the store since the read above.
                        let version = compatible_version(&transaction)?;
                        transaction.execute_batch(SCHEMA)?;
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
                super::frames::forget_snapshot(path);
            }
        }
        let id: Option<i64> = connection.query_row("SELECT id FROM store_identity WHERE singleton = 1", [], |row| row.get(0)).optional()?;
        if let Some(id) = id {
            return Ok(id);
        }
        // A copy of a store published without an identity (the tests' template, `corpus::test_store`): it becomes a
        // store of its own at its first open, as a store made here does, by a writer's thread.
        rerail_accel::exec::scoped(|scope| {
            scope
                .spawn(|| -> StoreResult<()> {
                    super::hazards::behind_the_run();
                    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                    transaction.execute("INSERT OR IGNORE INTO store_identity(singleton, id) VALUES (1, random())", [])?;
                    transaction.commit()?;
                    Ok(())
                })
                .join()
                .unwrap_or_else(|p| std::panic::resume_unwind(p))
        })?;
        if let Some(path) = connection.path() {
            super::atoms::forget_identity(path);
        }
        Ok(connection.query_row("SELECT id FROM store_identity WHERE singleton = 1", [], |row| row.get(0))?)
    }
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
    Ok(expected.iter().all(|object| present.contains(object)))
}

/// Every object SCHEMA creates, as `(type, name)`: read once from SCHEMA applied to an in-memory
/// database, so it cannot drift from SCHEMA.
/// Made on a writer's thread (decision 0104: a run's own threads only read).
static SCHEMA_OBJECTS: LazyLock<Result<Vec<(String, String)>, String>> = LazyLock::new(|| {
    rerail_accel::exec::scoped(|scope| {
        scope
            .spawn(|| {
                super::hazards::behind_the_run();
                let connection = Connection::open_in_memory().map_err(|error| error.to_string())?;
                connection.execute_batch(SCHEMA).map_err(|error| error.to_string())?;
                schema_object_names(&connection).map_err(|error| error.to_string())
            })
            .join()
            .unwrap_or_else(|p| std::panic::resume_unwind(p))
    })
});

fn schema_object_names(connection: &Connection) -> StoreResult<Vec<(String, String)>> {
    connection
        .prepare("SELECT type,name FROM sqlite_schema")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::from)
}
