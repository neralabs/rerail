use super::publication::{AFTER_PUBLICATION_READ_HOOK, BEFORE_RELATIONSHIP_WRITE_HOOK};
use super::read_cache::CacheKey;
use super::reclamation::{
    EXECUTED_COLLECT_SQL, STORE_ABSOLUTE_LIMIT, STORE_HARD_LIMIT, STORE_SOFT_LIMIT, capacity_state,
};
use super::rows::hash_params;
use super::schema::{SCHEMA_V1, SCHEMA_VERSION, STORE_REVISION};
use super::*;
use rusqlite::limits::Limit;
use rusqlite::params;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::Cursor;
use std::sync::Arc;
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

fn temporary_database(label: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "rerail-content-store-{label}-{}-{nonce}.sqlite",
        std::process::id()
    ))
}

fn store(label: &str) -> (std::path::PathBuf, ContentStore) {
    let path = temporary_database(label);
    let store = ContentStore::open(&path).unwrap();
    (path, store)
}

fn candidate_catalog(label: &str) -> CandidateCatalogIdentity {
    CandidateCatalogIdentity::new(
        HashKey::sha256(label.as_bytes()),
        HashKey::sha256(b"grammar-set"),
        1,
        1,
    )
    .unwrap()
}

/// Publishes `id` as a complete empty Git tree, so a snapshot may use it as its root.
fn published_tree(store: &ContentStore, id: HashKey) -> HashKey {
    store
        .put_git_trees_bulk(&[GitTree {
            id,
            prefix: String::new(),
            entries: Vec::new(),
        }])
        .unwrap();
    id
}

fn local_result_workspace(
    store: &ContentStore,
    label: &str,
) -> (WorkspaceId, CandidateCatalogIdentity) {
    let snapshot = store
        .put_snapshot(
            published_tree(store, HashKey::git_sha1([17; 20])),
            1,
            HashKey::sha256(b"grammar"),
            1,
        )
        .unwrap();
    let workspace = store
        .open_workspace(&format!("/{label}"), "repo", "owner", snapshot, 1, 100)
        .unwrap();
    (workspace, candidate_catalog(label))
}

#[test]
fn sparse_wal_enforces_real_capacity_states_and_write_rejection() {
    let (path, store) = store("disk-pressure");
    let wal = std::path::PathBuf::from(format!("{}-wal", path.display()));
    let base = fs::metadata(&path).unwrap().len();
    let set_total = |total: u64| {
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&wal)
            .unwrap();
        file.set_len(total.saturating_sub(base)).unwrap();
    };
    set_total(STORE_SOFT_LIMIT);
    assert_eq!(store.capacity().unwrap().state, CapacityState::Collect);
    set_total(STORE_HARD_LIMIT);
    assert_eq!(store.capacity().unwrap().state, CapacityState::RejectWrites);
    assert!(matches!(
        store.put_source(b"rejected", &[]),
        Err(StoreError::Invalid(_))
    ));
    set_total(STORE_ABSOLUTE_LIMIT);
    assert_eq!(store.capacity().unwrap().state, CapacityState::OverBudget);
    assert!(matches!(
        store.put_source(b"rejected", &[]),
        Err(StoreError::Invalid(_))
    ));
}

/// `SCAN` steps of `sql`'s query plan that run once per outer row of a correlated subquery.
fn correlated_scans(connection: &Connection, sql: &str) -> Vec<String> {
    let mut statement = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap();
    let unbound = vec![rusqlite::types::Null; statement.parameter_count()];
    let plan = statement
        .query_map(rusqlite::params_from_iter(unbound), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(3)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(!plan.is_empty(), "no query plan for {sql}");
    let steps = plan
        .iter()
        .map(|(id, parent, detail)| (*id, (*parent, detail.as_str())))
        .collect::<HashMap<_, _>>();
    plan.iter()
        .filter(|(_, parent, detail)| {
            let mut ancestor = *parent;
            detail.starts_with("SCAN ")
                && std::iter::from_fn(|| {
                    let (parent, detail) = steps.get(&ancestor)?;
                    ancestor = *parent;
                    Some(*detail)
                })
                .any(|detail| detail.starts_with("CORRELATED "))
        })
        .map(|(_, _, detail)| detail.clone())
        .collect()
}

#[test]
fn collect_statements_never_scan_inside_correlated_subqueries() {
    let (_path, store) = store("collect-plans");
    let tree = HashKey::git_sha1([41; 20]);
    store
        .put_git_trees_bulk(&[GitTree {
            id: tree,
            prefix: String::new(),
            entries: vec![GitTreeEntry {
                name: "source.py".to_owned(),
                mode: 0o100644,
                target: HashKey::git_sha1([42; 20]),
            }],
        }])
        .unwrap();
    let snapshot = store
        .put_snapshot(tree, 1, HashKey::sha256(b"grammar"), 1)
        .unwrap();
    let source = store.put_source(b"source", &[]).unwrap();
    let tape = store.put_tape(source, 1, 1, 1, b"tape").unwrap();
    store
        .put_snapshot_file(snapshot, "source.py", source, Some(tape))
        .unwrap();
    let workspace = store
        .open_workspace("/collect-plans", "repository", "owner", snapshot, 1, 100)
        .unwrap();
    store.retire_workspace(workspace).unwrap();

    EXECUTED_COLLECT_SQL.with(|executed| executed.borrow_mut().clear());
    let (mut snapshot_files, mut snapshots) = (0, 0);
    for _ in 0..4 {
        let report = store.collect(101, 100).unwrap();
        snapshot_files += report.snapshot_files;
        snapshots += report.snapshots;
    }
    // Both snapshot branches ran, so the recorder holds every statement collect can execute.
    assert!(snapshot_files > 0 && snapshots > 0);
    let executed = EXECUTED_COLLECT_SQL
        .with(|executed| executed.borrow().iter().copied().collect::<BTreeSet<_>>());
    let connection = store.connection.lock().unwrap();
    let scans = executed
        .iter()
        .flat_map(|sql| {
            correlated_scans(&connection, sql)
                .into_iter()
                .map(move |scan| format!("{scan} in {sql}"))
        })
        .collect::<Vec<_>>();
    assert!(
        scans.is_empty(),
        "collect scans a table once per outer row:\n{}",
        scans.join("\n")
    );
}

#[test]
fn hash_lookup_index_contains_only_algorithm_and_first_lane() {
    let (_path, store) = store("hash-prefix-index");
    let connection = store.connection.lock().unwrap();
    let indexes = connection
        .prepare("PRAGMA index_list(hashes)")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(indexes, vec!["hashes_by_prefix"]);
    let columns = connection
        .prepare("PRAGMA index_info(hashes_by_prefix)")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(2))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(columns, vec!["algorithm_id", "lane0"]);
}

#[test]
fn bulk_tapes_validate_frames_deduplicate_and_preserve_cache_status() {
    let (_, store) = store("bulk-tapes");
    let source = store.put_source(b"source", &[]).unwrap();
    let first = store.put_tape(source, 1, 1, 1, b"first tape").unwrap();
    let second = store.put_tape(source, 2, 1, 1, b"second tape").unwrap();
    let loaded = store
        .get_tapes_bulk(&[first, second, first, TapeId(i64::MAX)])
        .unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(&**loaded[&first].0, b"first tape");
    assert_eq!(&**loaded[&second].0, b"second tape");
    let cached = store.get_tapes_bulk(&[first, second]).unwrap();
    assert!(cached.values().all(|(_, hit)| *hit));
    assert!(store.get_tapes_bulk(&[]).unwrap().is_empty());
}

#[test]
fn selector_fts_presence_is_revision_scoped_and_syntax_aware() {
    let (_, store) = store("search-fts");
    let source = store.put_source(b"execute commit", &[]).unwrap();
    let first = store.put_tape(source, 1, 1, 1, b"first tape").unwrap();
    let stale = store.put_tape(source, 2, 1, 1, b"second tape").unwrap();
    store
        .put_search_indexes_bulk(&[TapeSearchIndex {
            tape_id: first,
            index_version: 3,
            node_count: 4,
            has_errors: false,
            kind_tokens: "k002a k0001 k002a k002a".to_owned(),
        }])
        .unwrap();
    let lookups = [
        SearchKindLookup {
            tape_id: first,
            index_version: 3,
            kind_tokens: vec!["k002a".to_owned(), "k0002".to_owned()],
        },
        SearchKindLookup {
            tape_id: first,
            index_version: 3,
            kind_tokens: vec!["k0003".to_owned()],
        },
        SearchKindLookup {
            tape_id: stale,
            index_version: 3,
            kind_tokens: vec!["k002a".to_owned()],
        },
    ];
    assert_eq!(
        store.get_search_kind_presence_bulk(&lookups).unwrap(),
        vec![
            StoredKindPresence::Present,
            StoredKindPresence::Absent,
            StoredKindPresence::Unknown,
        ]
    );
    let connection = store.connection.lock().unwrap();
    let rows: i64 = connection
        .query_row("SELECT count(*) FROM search_index_sets", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(rows, 1);
}

#[test]
fn search_index_backlog_uses_a_tape_id_keyset_seek() {
    let (_, store) = store("search-index-keyset");
    let source = store.put_source(b"source", &[]).unwrap();
    for index in 0..128 {
        store
            .put_tape(source, index, 1, 1, format!("tape-{index}").as_bytes())
            .unwrap();
    }
    let connection = store.connection.lock().unwrap();
    let plan = connection
        .prepare(&format!(
            "EXPLAIN QUERY PLAN {}",
            super::search_index::TAPES_WITHOUT_SEARCH_INDEX_SQL
        ))
        .unwrap()
        .query_map(params![1_i64, 1_i64, 0_i64, 64_i64], |row| {
            row.get::<_, String>(3)
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        plan.iter()
            .any(|detail| detail.starts_with("SEARCH t USING INTEGER PRIMARY KEY (rowid>?)")),
        "{plan:?}"
    );
    assert!(
        !plan.iter().any(|detail| detail.starts_with("SCAN")),
        "{plan:?}"
    );
}

#[test]
fn bulk_removal_is_scoped_and_idempotent() {
    let (_, store) = store("bulk-removal");
    let snapshot = store
        .put_snapshot(
            published_tree(&store, HashKey::git_sha1([71; 20])),
            1,
            HashKey::sha256(b"grammar"),
            1,
        )
        .unwrap();
    let left = store
        .open_workspace("/bulk/left", "repo", "left", snapshot, 1, 100)
        .unwrap();
    let right = store
        .open_workspace("/bulk/right", "repo", "right", snapshot, 1, 100)
        .unwrap();
    let source = store.put_source(b"x", &[]).unwrap();
    for owner in [left, right] {
        for path in ["a.py", "b.py"] {
            store
                .put_overlay(owner, path, 1, Some(source), None)
                .unwrap();
        }
    }
    let paths = vec![
        "a.py".to_owned(),
        "b.py".to_owned(),
        "missing.py".to_owned(),
    ];
    store.remove_overlay_files(left, &paths).unwrap();
    store.remove_overlay_files(left, &paths).unwrap();
    for path in ["a.py", "b.py"] {
        assert!(store.overlay_file(left, path).unwrap().is_none());
        assert!(store.overlay_file(right, path).unwrap().is_some());
    }
}

#[test]
fn capacity_thresholds_are_exact() {
    assert_eq!(capacity_state(STORE_SOFT_LIMIT - 1), CapacityState::Normal);
    assert_eq!(capacity_state(STORE_SOFT_LIMIT), CapacityState::Collect);
    assert_eq!(
        capacity_state(STORE_HARD_LIMIT),
        CapacityState::RejectWrites
    );
    assert_eq!(
        capacity_state(STORE_ABSOLUTE_LIMIT),
        CapacityState::OverBudget
    );
}

#[test]
fn release_memory_drops_process_local_read_caches() {
    let (_, store) = store("release-memory");
    let source = store.put_source(b"source", &[]).unwrap();
    let tape = store.put_tape(source, 1, 1, 1, b"tape").unwrap();
    assert_eq!(&*store.get_source(source).unwrap(), b"source");
    assert_eq!(&**store.get_tape(tape).unwrap(), b"tape");
    store.release_memory().unwrap();
    assert!(store.decompressed_cache.lock().unwrap().is_empty());
    assert!(store.local_result_cache.lock().unwrap().is_empty());
}

#[test]
fn hash_lanes_are_stable_and_sha1_is_padded() {
    let key = HashKey::sha256(b"abc");
    assert_eq!(key.algorithm, HASH_SHA256);
    assert_eq!(
        key.lanes[0],
        i64::from_be_bytes([0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea])
    );
    assert_eq!(
        key.lanes[3],
        i64::from_be_bytes([0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad])
    );
    let sha1 = HashKey::git_sha1([0x5a; 20]);
    assert_eq!(sha1.algorithm, HASH_GIT_SHA1);
    assert_eq!(
        sha1.lanes[2],
        i64::from_be_bytes([0x5a, 0x5a, 0x5a, 0x5a, 0, 0, 0, 0])
    );
    assert_eq!(sha1.lanes[3], 0);
}

#[test]
fn source_and_tape_objects_deduplicate_and_round_trip() {
    let (_path, store) = store("objects");
    let alias = HashKey::git_sha1([7; 20]);
    let first = store.put_source(b"print('hello')\n", &[alias]).unwrap();
    let second = store.put_source(b"print('hello')\n", &[alias]).unwrap();
    assert_eq!(first, second);
    assert_eq!(store.source_by_alias(alias).unwrap(), Some(first));
    assert_eq!(store.get_source(first).unwrap(), b"print('hello')\n");
    let tape = store.put_tape(first, 1, 45, 1, b"compact tape").unwrap();
    assert_eq!(
        store.put_tape(first, 1, 45, 1, b"compact tape").unwrap(),
        tape
    );
    assert_eq!(&**store.get_tape(tape).unwrap(), b"compact tape");
    {
        let cache = store.decompressed_cache.lock().unwrap();
        assert!(cache.contains(CacheKey::Source(HashKey::sha256(b"print('hello')\n"))));
        assert!(cache.contains(CacheKey::Tape(HashKey::sha256(b"compact tape"))));
    }
    {
        let connection = store.connection.lock().unwrap();
        let source_frame: (i64, Vec<u8>) = connection
            .query_row(
                "SELECT codec_id,frame FROM source_objects WHERE source_id=?1",
                [first.0],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let tape_frame: (i64, Vec<u8>) = connection
            .query_row(
                "SELECT codec_id,frame FROM tape_objects WHERE tape_id=?1",
                [tape.0],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(source_frame.0, CODEC_ZSTD);
        assert_eq!(tape_frame.0, CODEC_ZSTD);
        assert_eq!(
            zstd::stream::decode_all(Cursor::new(source_frame.1)).unwrap(),
            b"print('hello')\n"
        );
        assert_eq!(
            zstd::stream::decode_all(Cursor::new(tape_frame.1)).unwrap(),
            b"compact tape"
        );
    }
    assert!(matches!(
        store.put_tape(first, 1, 45, 1, b"different tape"),
        Err(StoreError::Corrupt(_))
    ));
}

#[test]
fn migrates_schema_one_to_schema_two() {
    let path = temporary_database("migration-v1-v2");
    let connection = Connection::open(&path).unwrap();
    connection.execute_batch(SCHEMA_V1).unwrap();
    connection.pragma_update(None, "user_version", 1).unwrap();
    drop(connection);

    let store = ContentStore::open(&path).unwrap();
    let connection = store.connection.lock().unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
    for table in [
        "candidate_catalogs",
        "candidate_checks",
        "local_check_results",
        "context_check_results",
        "completed_directory_reuse",
        "store_schema_revision",
        "search_index_sets",
        "search_kind_fts",
    ] {
        let found: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert!(found, "missing migrated table {table}");
    }
}
#[test]
fn schema_twelve_store_migrates_directory_reuse_and_additive_revision() {
    let path = temporary_database("migration-floor-v12");
    let store = ContentStore::open(&path).unwrap();
    drop(store);
    downgrade_to_floor_sixteen(&path);
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "DROP TABLE context_check_results;
                 DROP TABLE completed_directory_reuse;
                 DROP TABLE store_schema_revision;
                 ALTER TABLE search_index_sets DROP COLUMN has_errors;
                 CREATE VIRTUAL TABLE search_kind_instances USING fts5vocab(search_kind_fts, 'instance');
                 PRAGMA user_version=12;",
        )
        .unwrap();
    drop(connection);

    let store = ContentStore::open(&path).unwrap();
    let connection = store.connection.lock().unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    let revision: i64 = connection
        .query_row(
            "SELECT revision FROM store_schema_revision WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
    assert_eq!(revision, STORE_REVISION);
}

#[test]
fn recorded_store_revision_is_not_downgraded() {
    let (path, store) = store("revision-not-downgraded");
    drop(store);
    let connection = Connection::open(&path).unwrap();
    connection
        .execute(
            "UPDATE store_schema_revision SET revision=?1 WHERE singleton=1",
            [STORE_REVISION + 1],
        )
        .unwrap();
    drop(connection);

    let store = ContentStore::open(&path).unwrap();
    let connection = store.connection.lock().unwrap();
    let revision: i64 = connection
        .query_row(
            "SELECT revision FROM store_schema_revision WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(revision, STORE_REVISION + 1);
}

#[test]
fn newer_compatibility_floor_is_refused() {
    let (path, store) = store("newer-floor-refused");
    drop(store);
    let connection = Connection::open(&path).unwrap();
    connection
        .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
        .unwrap();
    drop(connection);

    assert!(matches!(
        ContentStore::open(&path),
        Err(StoreError::Invalid(message)) if message.contains("newer than supported")
    ));
}

#[test]
fn snapshot_publication_is_atomic() {
    let (_path, store) = store("atomic-snapshot");
    let snapshot = store
        .put_snapshot(
            published_tree(&store, HashKey::git_sha1([5; 20])),
            1,
            HashKey::sha256(b"grammar"),
            1,
        )
        .unwrap();
    let candidate = CandidateCheck {
        policy: HashKey::sha256(b"policy"),
        check: "check".to_owned(),
    };
    let stored = store
        .publish_snapshot_file(
            snapshot,
            SnapshotFilePublication {
                path: "src/a.py",
                source: b"x = 1\n",
                aliases: &[],
                tape: Some(TapePayload {
                    grammar_id: 1,
                    grammar_version: 45,
                    tape_format_version: 1,
                    bytes: b"persisted tape",
                }),
                catalog: candidate_catalog("atomic"),
                candidates: std::slice::from_ref(&candidate),
            },
        )
        .unwrap();
    assert_eq!(store.get_source(stored.source).unwrap(), b"x = 1\n");
    assert_eq!(
        &**store.get_tape(stored.tape.unwrap()).unwrap(),
        b"persisted tape"
    );

    let failed = store.publish_snapshot_file(
        SnapshotId(i64::MAX),
        SnapshotFilePublication {
            path: "src/missing.py",
            source: b"must roll back",
            aliases: &[],
            tape: None,
            catalog: candidate_catalog("failed"),
            candidates: &[],
        },
    );
    assert!(failed.is_err());
    let connection = store.connection.lock().unwrap();
    let leaked: i64 = connection
        .query_row(
            "SELECT count(*) FROM source_objects WHERE raw_length=?1",
            [b"must roll back".len() as i64],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(leaked, 0);

    let publication = SnapshotFilePublication {
        path: "src/b.py",
        source: b"x = 2\n",
        aliases: &[],
        tape: None,
        catalog: candidate_catalog("bounded"),
        candidates: &[],
    };
    assert!(matches!(
        store.publish_snapshot_files(snapshot, &vec![publication; MAX_PUBLICATION_BATCH + 1]),
        Err(StoreError::Invalid(_))
    ));
}

#[test]
fn overlay_publication_replaces_payload_and_candidates_atomically() {
    let (_path, store) = store("atomic-overlay");
    let snapshot = store
        .put_snapshot(
            published_tree(&store, HashKey::git_sha1([4; 20])),
            1,
            HashKey::sha256(b"grammar"),
            1,
        )
        .unwrap();
    let base = store.put_source(b"x = 1\n", &[]).unwrap();
    store
        .put_snapshot_file(snapshot, "src/a.py", base, None)
        .unwrap();
    let workspace = store
        .open_workspace("/atomic", "repo", "owner", snapshot, 1, 100)
        .unwrap();
    let candidate = CandidateCheck {
        policy: HashKey::sha256(b"policy"),
        check: "changed".to_owned(),
    };
    let tape_bytes = b"compressed tape payload";
    let publications = [
        OverlayFilePublication {
            path: "src/a.py",
            state_id: 2,
            source: b"x = 2\n",
            aliases: &[],
            tape: Some(TapePayload {
                grammar_id: 1,
                grammar_version: 1,
                tape_format_version: 1,
                bytes: tape_bytes,
            }),
            catalog: candidate_catalog("overlay-atomic"),
            candidates: std::slice::from_ref(&candidate),
            local_results: &[],
            candidates_current: false,
        },
        OverlayFilePublication {
            path: "src/b.py",
            state_id: 2,
            source: b"y = 3\n",
            aliases: &[],
            tape: None,
            catalog: candidate_catalog("overlay-atomic"),
            candidates: &[],
            local_results: &[],
            candidates_current: false,
        },
    ];
    let result = store
        .publish_overlay_files(workspace, &publications)
        .unwrap();
    assert_eq!(result.retries, 0);
    assert_eq!(result.reconciliation_passes, 2);
    assert!(result.data_statements <= 20);
    assert!(result.max_data_statements_per_pass <= 10);
    assert_eq!(result.compressed_sources, 2);
    assert_eq!(result.compressed_tapes, 1);
    assert_eq!(result.files.len(), 2);
    assert_eq!(
        store.get_source(result.files[0].source).unwrap(),
        b"x = 2\n"
    );
    assert_eq!(
        store.get_source(result.files[1].source).unwrap(),
        b"y = 3\n"
    );
    assert_eq!(
        &**store.get_tape(result.files[0].tape.unwrap()).unwrap(),
        tape_bytes
    );
    let reused = store
        .publish_overlay_files(workspace, &publications)
        .unwrap();
    assert_eq!(reused.reused_sources, 2);
    assert_eq!(reused.reused_tapes, 1);
    assert_eq!(reused.reconciliation_passes, 1);
    assert!(reused.max_data_statements_per_pass <= 10);
    assert_eq!(reused.compressed_sources, 0);
    assert_eq!(reused.compressed_tapes, 0);
    let (codec, frame): (i64, Vec<u8>) = store
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT codec_id,frame FROM source_objects WHERE source_id=?1",
            [result.files[0].source.0],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(codec, CODEC_ZSTD);
    assert_eq!(
        zstd::stream::decode_all(Cursor::new(frame)).unwrap(),
        b"x = 2\n"
    );
}

#[test]
fn local_results_publish_across_sqlite_variable_limit_chunks() {
    let (_path, store) = store("bulk-local-results");
    store
        .connection
        .lock()
        .unwrap()
        .set_limit(Limit::SQLITE_LIMIT_VARIABLE_NUMBER, 100)
        .unwrap();
    let snapshot = store
        .put_snapshot(
            published_tree(&store, HashKey::git_sha1([3; 20])),
            1,
            HashKey::sha256(b"grammar"),
            1,
        )
        .unwrap();
    let workspace = store
        .open_workspace("/bulk-results", "repo", "owner", snapshot, 1, 100)
        .unwrap();
    let catalog = candidate_catalog("bulk-results");
    let results = (0..12)
        .map(|index| {
            let bytes = format!("result-{index}").into_bytes();
            LocalResultPublication {
                candidate: CandidateCheck {
                    policy: HashKey::sha256(format!("policy-{index}").as_bytes()),
                    check: format!("check-{index}"),
                },
                context: HashKey::sha256(b"bulk-context"),
                raw_length: bytes.len(),
                frame: zstd::stream::encode_all(Cursor::new(bytes), 3).unwrap(),
            }
        })
        .collect::<Vec<_>>();
    let publication = OverlayFilePublication {
        path: "src/results.py",
        state_id: 1,
        source: b"pass\n",
        aliases: &[],
        tape: None,
        catalog,
        candidates: &[],
        local_results: &results,
        candidates_current: false,
    };
    let stored = store.publish_overlay_file(workspace, publication).unwrap();
    let restored = store
        .local_check_results(stored.source, HashKey::sha256(b"bulk-context"), catalog)
        .unwrap();
    assert_eq!(restored.len(), results.len());
    assert!(restored.iter().any(|result| result.bytes == b"result-0"));
    assert!(restored.iter().any(|result| result.bytes == b"result-11"));
}

#[test]
fn relationship_revalidation_does_not_hash_payloads_under_write_lock() {
    let (_path, store) = store("relationship-prepared-hashes");
    let (workspace, catalog) = local_result_workspace(&store, "relationship-prepared-hashes");
    take_write_lock_hash_counts();
    store
        .publish_overlay_file(
            workspace,
            OverlayFilePublication {
                path: "src/locked.py",
                state_id: 1,
                source: b"pass\n",
                aliases: &[],
                tape: None,
                catalog,
                candidates: &[],
                local_results: &[],
                candidates_current: false,
            },
        )
        .unwrap();
    let (write_sections, hashes_under_lock) = take_write_lock_hash_counts();
    assert_eq!(
        write_sections, 2,
        "the object and relationship write transactions were not both observed"
    );
    assert_eq!(
        hashes_under_lock, 0,
        "SHA-256 was computed while publication held the write lock"
    );
}
#[test]
fn local_results_are_scoped_by_context_hash() {
    let (_path, store) = store("local-result-contexts");
    let (workspace, catalog) = local_result_workspace(&store, "local-result-contexts");
    let candidate = CandidateCheck {
        policy: HashKey::sha256(b"policy"),
        check: "check".to_owned(),
    };
    let first_context = HashKey::sha256(b"alpha/x.py");
    let second_context = HashKey::sha256(b"beta/x.py");
    let first = LocalResultPublication {
        candidate: candidate.clone(),
        context: first_context,
        raw_length: 5,
        frame: zstd::stream::encode_all(Cursor::new(b"alpha"), 3).unwrap(),
    };
    let second = LocalResultPublication {
        candidate,
        context: second_context,
        raw_length: 4,
        frame: zstd::stream::encode_all(Cursor::new(b"beta"), 3).unwrap(),
    };
    let stored = store
        .publish_overlay_file(
            workspace,
            OverlayFilePublication {
                path: "x.py",
                state_id: 1,
                source: b"pass\n",
                aliases: &[],
                tape: None,
                catalog,
                candidates: &[],
                local_results: &[first, second],
                candidates_current: false,
            },
        )
        .unwrap();

    assert_eq!(
        store
            .local_check_results(stored.source, first_context, catalog)
            .unwrap()[0]
            .bytes,
        b"alpha"
    );
    assert_eq!(
        store
            .local_check_results(stored.source, second_context, catalog)
            .unwrap()[0]
            .bytes,
        b"beta"
    );
    let by_key = store
        .local_check_results_by_key(
            &[
                (HashKey::sha256(b"pass\n"), first_context),
                (HashKey::sha256(b"pass\n"), second_context),
            ],
            catalog,
        )
        .unwrap();
    assert_eq!(
        by_key[&(HashKey::sha256(b"pass\n"), first_context)][0].bytes,
        b"alpha"
    );
    assert_eq!(
        by_key[&(HashKey::sha256(b"pass\n"), second_context)][0].bytes,
        b"beta"
    );
}

#[test]
fn a_result_under_a_new_context_of_stored_objects_is_kept() {
    let (_path, store) = store("local-result-new-context");
    let (workspace, catalog) = local_result_workspace(&store, "local-result-new-context");
    let result = |context: &[u8], bytes: &'static [u8]| LocalResultPublication {
        candidate: CandidateCheck {
            policy: HashKey::sha256(b"policy"),
            check: "check".to_owned(),
        },
        context: HashKey::sha256(context),
        raw_length: bytes.len(),
        frame: zstd::stream::encode_all(Cursor::new(bytes), 3).unwrap(),
    };
    let publish = |results: &[LocalResultPublication]| {
        store
            .publish_overlay_file(
                workspace,
                OverlayFilePublication {
                    path: "x.py",
                    state_id: 1,
                    source: b"pass\n",
                    aliases: &[],
                    tape: None,
                    catalog,
                    candidates: &[],
                    local_results: results,
                    candidates_current: false,
                },
            )
            .unwrap()
    };
    publish(&[result(b"first", b"alpha")]);
    // Source, path, catalog and check are all stored now: only the context is new.
    let stored = publish(&[result(b"second", b"beta")]);
    let restored = store
        .local_check_results(stored.source, HashKey::sha256(b"second"), catalog)
        .unwrap();
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].bytes, b"beta");
}

#[test]
fn context_results_cascade_when_sources_and_checks_are_deleted() {
    for delete_source in [true, false] {
        let (_path, store) = store(if delete_source {
            "context-result-source-cascade"
        } else {
            "context-result-check-cascade"
        });
        let (workspace, catalog) = local_result_workspace(&store, "context-result-cascade");
        let result = LocalResultPublication {
            candidate: CandidateCheck {
                policy: HashKey::sha256(b"policy"),
                check: "check".to_owned(),
            },
            context: HashKey::sha256(b"context"),
            raw_length: 6,
            frame: zstd::stream::encode_all(Cursor::new(b"result"), 3).unwrap(),
        };
        let stored = store
            .publish_overlay_file(
                workspace,
                OverlayFilePublication {
                    path: "x.py",
                    state_id: 1,
                    source: b"pass\n",
                    aliases: &[],
                    tape: None,
                    catalog,
                    candidates: &[],
                    local_results: &[result],
                    candidates_current: false,
                },
            )
            .unwrap();
        let connection = store.connection.lock().unwrap();
        if delete_source {
            connection
                .execute(
                    "DELETE FROM workspace_overlay WHERE workspace_id=?1",
                    [workspace.0],
                )
                .unwrap();
            connection
                .execute(
                    "DELETE FROM source_objects WHERE source_id=?1",
                    [stored.source.0],
                )
                .unwrap();
        } else {
            connection
                .execute(
                    "DELETE FROM candidate_checks WHERE check_id=(SELECT check_id FROM context_check_results LIMIT 1)",
                    [],
                )
                .unwrap();
        }
        let count: i64 = connection
            .query_row("SELECT count(*) FROM context_check_results", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }
}

#[test]
fn reconciliation_reprepares_source_collected_after_the_read() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let (path, store) = store("publication-reprepare");
    let (workspace, catalog) = local_result_workspace(&store, "publication-reprepare");
    store.put_source(b"pass\n", &[]).unwrap();
    ContentStore::open(&path).unwrap().collect(0, 100).unwrap();
    *AFTER_PUBLICATION_READ_HOOK.lock() = Some(Box::new(move || {
        ContentStore::open(&path).unwrap().collect(0, 100).unwrap();
    }));
    let polls = AtomicUsize::new(0);
    let result = store.publish_overlay_files_cancellable(
        workspace,
        &[OverlayFilePublication {
            path: "src/reprepare.py",
            state_id: 1,
            source: b"pass\n",
            aliases: &[],
            tape: None,
            catalog,
            candidates: &[],
            local_results: &[],
            candidates_current: false,
        }],
        &|| polls.fetch_add(1, Ordering::SeqCst) >= 5,
    );
    assert!(
        result.is_ok(),
        "publication must reconcile a collected source: {result:?}"
    );
}

#[test]
fn reconciliation_revalidates_check_collected_before_relationship_write() {
    let (path, store) = store("publication-revalidate-check");
    let (workspace, catalog) = local_result_workspace(&store, "publication-revalidate-check");
    let candidate = CandidateCheck {
        policy: HashKey::sha256(b"policy"),
        check: "check".to_owned(),
    };
    let result = LocalResultPublication {
        candidate: candidate.clone(),
        context: HashKey::sha256(b"context"),
        raw_length: 6,
        frame: zstd::stream::encode_all(Cursor::new(b"result"), 3).unwrap(),
    };
    *BEFORE_RELATIONSHIP_WRITE_HOOK.lock() = Some(Box::new(move || {
        ContentStore::open(&path).unwrap().collect(0, 100).unwrap();
    }));
    let stored = store
        .publish_overlay_file(
            workspace,
            OverlayFilePublication {
                path: "src/revalidate.py",
                state_id: 1,
                source: b"pass\n",
                aliases: &[],
                tape: None,
                catalog,
                candidates: &[],
                local_results: &[result],
                candidates_current: false,
            },
        )
        .unwrap();
    assert_eq!(
        store
            .local_check_results(stored.source, HashKey::sha256(b"context"), catalog)
            .unwrap()
            .into_iter()
            .map(|result| result.candidate)
            .collect::<Vec<_>>(),
        vec![candidate]
    );
}

#[test]
fn reconciliation_revalidates_path_collected_before_relationship_write() {
    let (path, store) = store("publication-revalidate-path");
    let (workspace, catalog) = local_result_workspace(&store, "publication-revalidate-path");
    *BEFORE_RELATIONSHIP_WRITE_HOOK.lock() = Some(Box::new(move || {
        let connection = Connection::open(path).unwrap();
        connection
            .execute(
                "DELETE FROM paths WHERE path_id=(SELECT p.path_id FROM paths p \
                 JOIN strings s ON s.string_id=p.string_id WHERE s.value=?1)",
                ["src/revalidate-path.py"],
            )
            .unwrap();
    }));
    store
        .publish_overlay_file(
            workspace,
            OverlayFilePublication {
                path: "src/revalidate-path.py",
                state_id: 1,
                source: b"pass\n",
                aliases: &[],
                tape: None,
                catalog,
                candidates: &[],
                local_results: &[],
                candidates_current: false,
            },
        )
        .unwrap();
    assert!(
        store
            .overlay_file(workspace, "src/revalidate-path.py")
            .unwrap()
            .is_some()
    );
}

#[test]
fn warm_overlay_publication_stores_new_git_aliases() {
    let (_path, store) = store("warm-publication-alias");
    let (workspace, catalog) = local_result_workspace(&store, "warm-publication-alias");
    let publication = |aliases| OverlayFilePublication {
        path: "src/alias.py",
        state_id: 1,
        source: b"pass\n",
        aliases,
        tape: None,
        catalog,
        candidates: &[],
        local_results: &[],
        candidates_current: false,
    };
    let stored = store
        .publish_overlay_file(workspace, publication(&[]))
        .unwrap();
    let alias = HashKey::git_sha1([42; 20]);
    store
        .publish_overlay_file(workspace, publication(&[alias]))
        .unwrap();
    assert_eq!(store.source_by_alias(alias).unwrap(), Some(stored.source));
}

#[test]
fn cancelled_overlay_publication_rolls_back_before_commit() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let (_path, store) = store("cancelled-overlay");
    let snapshot = store
        .put_snapshot(
            published_tree(&store, HashKey::git_sha1([7; 20])),
            1,
            HashKey::sha256(b"grammar"),
            1,
        )
        .unwrap();
    let workspace = store
        .open_workspace("/cancelled", "repo", "owner", snapshot, 1, 100)
        .unwrap();
    let polls = AtomicUsize::new(0);
    let publication = OverlayFilePublication {
        path: "src/a.py",
        state_id: 1,
        source: b"x = 1\n",
        aliases: &[],
        tape: None,
        catalog: candidate_catalog("cancelled"),
        candidates: &[],
        local_results: &[],
        candidates_current: false,
    };
    let result = store.publish_overlay_files_cancellable(workspace, &[publication], &|| {
        polls.fetch_add(1, Ordering::SeqCst) >= 2
    });
    assert!(matches!(result, Err(StoreError::Invalid(_))));
    assert!(store.overlay_file(workspace, "src/a.py").unwrap().is_none());
    let stored_source_count: i64 = store
        .connection
        .lock()
        .unwrap()
        .query_row("SELECT count(*) FROM source_objects", [], |row| row.get(0))
        .unwrap();
    assert_eq!(stored_source_count, 1);
}

#[test]
fn overlay_publication_reports_busy_retries() {
    let (path, store) = store("overlay-retries");
    let snapshot = store
        .put_snapshot(
            published_tree(&store, HashKey::git_sha1([8; 20])),
            1,
            HashKey::sha256(b"grammar"),
            1,
        )
        .unwrap();
    let workspace = store
        .open_workspace("/retry", "repo", "owner", snapshot, 1, 100)
        .unwrap();
    let blocker = Connection::open(&path).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let publisher = std::thread::spawn(move || {
        store
            .publish_overlay_files(
                workspace,
                &[OverlayFilePublication {
                    path: "src/a.py",
                    state_id: 1,
                    source: b"x = 1\n",
                    aliases: &[],
                    tape: None,
                    catalog: candidate_catalog("retry"),
                    candidates: &[],
                    local_results: &[],
                    candidates_current: false,
                }],
            )
            .unwrap()
    });
    std::thread::sleep(Duration::from_millis(600));
    blocker.execute_batch("COMMIT").unwrap();
    let result = publisher.join().unwrap();
    assert!(result.retries >= 1);
    assert_eq!(result.files.len(), 1);
}

#[test]
fn completed_directories_require_matching_package_markers() {
    let (_path, store) = store("directory-reuse");
    let tree = published_tree(&store, HashKey::git_sha1([31; 20]));
    let snapshot = store
        .put_snapshot(tree, 1, HashKey::sha256(b"grammar"), 1)
        .unwrap();
    let workspace = store
        .open_workspace("/repo/directory-reuse", "repo", "owner", snapshot, 1, 100)
        .unwrap();
    let catalog = candidate_catalog("directory-reuse");
    let completed = DirectoryReuse {
        prefix: "src".to_owned(),
        tree,
        package_markers: HashKey::sha256(b"without-marker"),
    };
    store
        .complete_directories(
            WorkspaceCompletion {
                workspace_id: workspace,
                revision: 2,
                completed_at: 200,
            },
            catalog,
            std::slice::from_ref(&completed),
        )
        .unwrap();
    assert_eq!(
        store.completed_workspace_revision(workspace).unwrap(),
        Some(2)
    );
    assert_eq!(
        store
            .reusable_directory_prefixes(catalog, std::slice::from_ref(&completed))
            .unwrap(),
        HashSet::from(["src".to_owned()])
    );
    let changed_markers = DirectoryReuse {
        package_markers: HashKey::sha256(b"with-marker"),
        ..completed
    };
    assert!(
        store
            .reusable_directory_prefixes(catalog, &[changed_markers])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn snapshots_are_shared_and_workspace_overlays_are_isolated() {
    let (_path, store) = store("snapshots");
    let tree = published_tree(&store, HashKey::git_sha1([1; 20]));
    let grammar = HashKey::sha256(b"python-grammar");
    let snapshot = store.put_snapshot(tree, 1, grammar, 1).unwrap();
    assert_eq!(store.put_snapshot(tree, 1, grammar, 1).unwrap(), snapshot);
    let source = store.put_source(b"x = 1\n", &[]).unwrap();
    let tape = store.put_tape(source, 1, 45, 1, b"tape").unwrap();
    store
        .put_snapshot_file(snapshot, "src/a.py", source, Some(tape))
        .unwrap();
    let left = store
        .open_workspace("/repo/left", "repo", "left-owner", snapshot, 1, 100)
        .unwrap();
    let right = store
        .open_workspace("/repo/right", "repo", "right-owner", snapshot, 1, 100)
        .unwrap();
    assert_ne!(left, right);
    store
        .renew_workspace_lease(left, "left-owner", 200)
        .unwrap();
    assert!(
        store
            .renew_workspace_lease(left, "wrong-owner", 300)
            .is_err()
    );
    assert_eq!(store.capacity().unwrap().state, CapacityState::Normal);
    let changed = store.put_source(b"x = 2\n", &[]).unwrap();
    store
        .put_overlay(left, "src/a.py", 1, Some(changed), None)
        .unwrap();
    assert_eq!(store.get_source(source).unwrap(), b"x = 1\n");
    assert_eq!(store.get_source(changed).unwrap(), b"x = 2\n");
}

#[test]
fn collection_keeps_shared_objects_until_the_last_workspace_retires() {
    let (_path, store) = store("collection");
    let snapshot = store
        .put_snapshot(
            published_tree(&store, HashKey::git_sha1([2; 20])),
            1,
            HashKey::sha256(b"grammar"),
            1,
        )
        .unwrap();
    let source = store.put_source(b"shared\n", &[]).unwrap();
    let tape = store.put_tape(source, 1, 1, 1, b"tape").unwrap();
    store
        .put_snapshot_file(snapshot, "shared.py", source, Some(tape))
        .unwrap();
    let left = store
        .open_workspace("/left", "repo", "owner-left", snapshot, 1, 100)
        .unwrap();
    let right = store
        .open_workspace("/right", "repo", "owner-right", snapshot, 1, 100)
        .unwrap();
    store.retire_workspace(left).unwrap();
    for _ in 0..4 {
        store.collect(0, 100).unwrap();
    }
    assert_eq!(store.get_source(source).unwrap(), b"shared\n");
    store.retire_workspace(right).unwrap();
    for _ in 0..8 {
        store.collect(101, 100).unwrap();
    }
    assert!(matches!(
        store.get_source(source),
        Err(StoreError::Database(rusqlite::Error::QueryReturnedNoRows))
    ));
}

#[test]
fn corrupt_zstandard_frames_fail_closed() {
    let (_path, store) = store("corrupt-frames");
    let source = store.put_source(b"source", &[]).unwrap();
    let tape = store.put_tape(source, 1, 1, 1, b"tape").unwrap();
    {
        let connection = store.connection.lock().unwrap();
        connection
            .execute(
                "UPDATE source_objects SET frame=x'000102' WHERE source_id=?1",
                [source.0],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE tape_objects SET frame=x'000102' WHERE tape_id=?1",
                [tape.0],
            )
            .unwrap();
    }
    assert!(store.get_source(source).is_err());
    assert!(store.get_tape(tape).is_err());
}

#[test]
fn multiprocess_publish_helper() {
    let Ok(path) = std::env::var("RERAIL_MULTIPROCESS_STORE") else {
        return;
    };
    let workspace_id = WorkspaceId(
        std::env::var("RERAIL_MULTIPROCESS_WORKSPACE")
            .unwrap()
            .parse()
            .unwrap(),
    );
    let index = std::env::var("RERAIL_MULTIPROCESS_INDEX").unwrap();
    let store = ContentStore::open(Path::new(&path)).unwrap();
    store
        .publish_overlay_files(
            workspace_id,
            &[OverlayFilePublication {
                path: &format!("src/{index}.py"),
                state_id: 1,
                source: b"shared source\n",
                aliases: &[],
                tape: None,
                catalog: candidate_catalog("multiprocess"),
                candidates: &[],
                local_results: &[],
                candidates_current: false,
            }],
        )
        .unwrap();
}

#[test]
fn multiple_processes_converge_while_publishing() {
    let (path, store) = store("multiprocess");
    let snapshot = store
        .put_snapshot(
            published_tree(&store, HashKey::git_sha1([8; 20])),
            1,
            HashKey::sha256(b"grammar"),
            1,
        )
        .unwrap();
    let workspaces = (0..4)
        .map(|index| {
            store
                .open_workspace(
                    &format!("/multiprocess/{index}"),
                    "repo",
                    &format!("owner-{index}"),
                    snapshot,
                    1,
                    100,
                )
                .unwrap()
        })
        .collect::<Vec<_>>();
    drop(store);
    let executable = std::env::current_exe().unwrap();
    let children = workspaces
        .iter()
        .enumerate()
        .map(|(index, workspace)| {
            std::process::Command::new(&executable)
                .args([
                    "--exact",
                    "content_store::tests::multiprocess_publish_helper",
                    "--nocapture",
                ])
                .env("RERAIL_MULTIPROCESS_STORE", &path)
                .env("RERAIL_MULTIPROCESS_WORKSPACE", workspace.0.to_string())
                .env("RERAIL_MULTIPROCESS_INDEX", index.to_string())
                .spawn()
                .unwrap()
        })
        .collect::<Vec<_>>();
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    let store = ContentStore::open(&path).unwrap();
    let source_count: i64 = store
        .connection
        .lock()
        .unwrap()
        .query_row("SELECT count(*) FROM source_objects", [], |row| row.get(0))
        .unwrap();
    assert_eq!(source_count, 1);
}

#[test]
fn concurrent_connections_converge_on_one_source() {
    let path = temporary_database("concurrent");
    let first = Arc::new(ContentStore::open(&path).unwrap());
    let second = Arc::new(ContentStore::open(&path).unwrap());
    let mut handles = Vec::new();
    for store in [first, second] {
        handles.push(thread::spawn(move || {
            for _ in 0..25 {
                match store.put_source(b"same bytes", &[]) {
                    Ok(id) => return id,
                    Err(StoreError::Busy) => thread::sleep(Duration::from_millis(2)),
                    Err(error) => panic!("unexpected store error: {error}"),
                }
            }
            panic!("writer stayed busy")
        }));
    }
    assert_eq!(
        handles.remove(0).join().unwrap(),
        handles.remove(0).join().unwrap()
    );
}

#[test]
fn one_store_uses_independent_thread_connections_for_reads_and_writes() {
    let (_path, store) = store("thread-connections");
    let source = store.put_source(b"shared", &[]).unwrap();
    let store = Arc::new(store);
    let writer = Arc::clone(&store);
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let handle = thread::spawn(move || {
        let connection = writer.connection.lock().unwrap();
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        started_tx.send(()).unwrap();
        thread::sleep(Duration::from_millis(150));
        connection.execute_batch("COMMIT").unwrap();
    });
    started_rx.recv().unwrap();
    let started = Instant::now();
    assert_eq!(store.get_source(source).unwrap(), b"shared");
    assert!(started.elapsed() < Duration::from_millis(100));
    handle.join().unwrap();
}

#[test]
fn one_snapshot_and_payload_serve_one_hundred_workspaces() {
    let (_path, store) = store("hundred-workspaces");
    let snapshot = store
        .put_snapshot(
            published_tree(&store, HashKey::git_sha1([9; 20])),
            1,
            HashKey::sha256(b"grammar-set"),
            1,
        )
        .unwrap();
    let source = store.put_source(b"shared source\n", &[]).unwrap();
    let tape = store.put_tape(source, 1, 1, 1, b"shared tape").unwrap();
    store
        .put_snapshot_file(snapshot, "src/shared.py", source, Some(tape))
        .unwrap();
    for index in 0..100 {
        store
            .open_workspace(
                &format!("/workspaces/{index}"),
                "shared-repository",
                &format!("owner-{index}"),
                snapshot,
                1,
                1_000,
            )
            .unwrap();
    }
    let connection = store.connection.lock().unwrap();
    for (table, expected) in [
        ("workspaces", 100_i64),
        ("snapshots", 1),
        ("snapshot_files", 1),
        ("source_objects", 1),
        ("tape_objects", 1),
    ] {
        let count = connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(count, expected, "unexpected {table} count");
    }
}

#[test]
fn concurrent_open_serializes_schema_creation() {
    let path = temporary_database("schema-race");
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let handles = (0..2)
        .map(|_| {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                ContentStore::open(&path).unwrap()
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        drop(handle.join().unwrap());
    }
}

#[test]
fn pins_protect_unpublished_objects() {
    let (_path, store) = store("pins");
    let source = store.put_source(b"pending", &[]).unwrap();
    let tape = store.put_tape(source, 1, 1, 1, b"pending tape").unwrap();
    store.pin_source(source, "indexer", 100).unwrap();
    store.pin_tape(tape, "indexer", 100).unwrap();
    for _ in 0..3 {
        store.collect(1, 100).unwrap();
    }
    assert_eq!(store.get_source(source).unwrap(), b"pending");
    assert_eq!(store.release_pins("indexer").unwrap(), 2);
    for _ in 0..3 {
        store.collect(1, 100).unwrap();
    }
    assert!(store.get_source(source).is_err());
}

#[test]
fn git_tree_entries_use_interned_integer_keys() {
    let (_path, store) = store("tree-entries");
    let tree = HashKey::git_sha1([3; 20]);
    let target = HashKey::git_sha1([4; 20]);
    store
        .put_git_tree_entry(tree, "source.py", 0o100644, 1, target)
        .unwrap();
    store
        .put_git_tree_entry(tree, "source.py", 0o100644, 1, target)
        .unwrap();
    let connection = store.connection.lock().unwrap();
    let count: i64 = connection
        .query_row("SELECT count(*) FROM git_tree_entries", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn workspace_leases_renew_in_one_bulk_statement() {
    let (_path, store) = store("lease");
    let snapshot = store
        .put_snapshot(
            published_tree(&store, HashKey::sha256(b"lease-snapshot")),
            1,
            HashKey::sha256(b"lease-grammars"),
            1,
        )
        .unwrap();
    let left = store
        .open_workspace("/lease-left", "repo", "one", snapshot, 1, 20)
        .unwrap();
    let right = store
        .open_workspace("/lease-right", "repo", "two", snapshot, 1, 20)
        .unwrap();
    assert_eq!(
        store
            .renew_workspace_leases(&[(left, "one"), (right, "two")], 40)
            .unwrap(),
        2
    );
    assert_eq!(
        store
            .renew_workspace_leases(&[(left, "wrong")], 50)
            .unwrap(),
        0
    );
}
#[test]
fn collection_in_another_store_cannot_alias_cached_object_ids() {
    let (path, first) = store("cross-store-cache-alias");
    let old_source = first.put_source(b"old source", &[]).unwrap();
    let old_tape = first.put_tape(old_source, 1, 1, 1, b"old tape").unwrap();
    assert_eq!(first.get_source(old_source).unwrap(), b"old source");
    assert_eq!(&**first.get_tape(old_tape).unwrap(), b"old tape");

    let second = ContentStore::open(&path).unwrap();
    second.collect(1, 100).unwrap();
    second.collect(1, 100).unwrap();
    let new_source = second.put_source(b"new source", &[]).unwrap();
    let new_tape = second.put_tape(new_source, 1, 1, 1, b"new tape").unwrap();
    assert_eq!(new_source, old_source);
    assert_eq!(new_tape, old_tape);

    assert_eq!(first.get_source(new_source).unwrap(), b"new source");
    assert_eq!(&**first.get_tape(new_tape).unwrap(), b"new tape");
}

#[test]
fn bulk_git_tree_publication_round_trips_entries() {
    let (_path, store) = store("bulk-tree-entries");
    let tree = HashKey::git_sha1([3; 20]);
    let entries = vec![
        GitTreeEntry {
            name: "source.py".to_owned(),
            mode: 0o100644,
            target: HashKey::git_sha1([4; 20]),
        },
        GitTreeEntry {
            name: "nested".to_owned(),
            mode: 0o040000,
            target: HashKey::git_sha1([5; 20]),
        },
    ];
    store
        .put_git_trees_bulk(&[GitTree {
            id: tree,
            prefix: String::new(),
            entries: entries.clone(),
        }])
        .unwrap();

    let connection = store.connection.lock().unwrap();
    let stored = connection
        .prepare(
            "SELECT s.value,e.mode_id,h.algorithm_id,h.lane0,h.lane1,h.lane2,h.lane3 \
             FROM git_tree_entries e JOIN git_trees t ON t.tree_id=e.tree_id \
             JOIN hashes th ON th.hash_id=t.tree_hash_id JOIN strings s ON s.string_id=e.name_string_id \
             JOIN hashes h ON h.hash_id=e.target_hash_id \
             WHERE th.algorithm_id=?1 AND th.lane0=?2 AND th.lane1=?3 AND th.lane2=?4 AND th.lane3=?5 \
             ORDER BY s.value",
        )
        .unwrap()
        .query_map(hash_params(tree), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                HashKey {
                    algorithm: row.get(2)?,
                    lanes: [row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?],
                },
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        stored,
        vec![
            ("nested".to_owned(), 0o040000, HashKey::git_sha1([5; 20])),
            ("source.py".to_owned(), 0o100644, HashKey::git_sha1([4; 20])),
        ]
    );
}

#[test]
fn unchanged_git_tree_publication_writes_no_entries() {
    let (_path, store) = store("bulk-tree-republish");
    let tree = GitTree {
        id: HashKey::git_sha1([12; 20]),
        prefix: String::new(),
        entries: vec![
            GitTreeEntry {
                name: "source.py".to_owned(),
                mode: 0o100644,
                target: HashKey::git_sha1([13; 20]),
            },
            GitTreeEntry {
                name: "other.py".to_owned(),
                mode: 0o100644,
                target: HashKey::git_sha1([14; 20]),
            },
        ],
    };
    store
        .put_git_trees_bulk(std::slice::from_ref(&tree))
        .unwrap();
    store.reset_git_tree_write_count();
    store.put_git_trees_bulk(&[tree]).unwrap();
    assert_eq!(store.git_tree_lookup_count(), 1);
    assert_eq!(store.git_tree_write_count(), 0);
}

#[test]
fn cold_git_tree_publication_batches_whole_trees_per_transaction() {
    let (_path, store) = store("bulk-tree-cold");
    let tree = |id: u32, entries: u32| {
        let mut bytes = [0_u8; 20];
        bytes[..4].copy_from_slice(&id.to_be_bytes());
        GitTree {
            id: HashKey::git_sha1(bytes),
            prefix: String::new(),
            entries: (0..entries)
                .map(|entry| {
                    let mut target = bytes;
                    target[4..8].copy_from_slice(&(entry + 1).to_be_bytes());
                    GitTreeEntry {
                        name: format!("file-{entry}"),
                        mode: 0o100644,
                        target: HashKey::git_sha1(target),
                    }
                })
                .collect(),
        }
    };
    let trees = (0..300).map(|id| tree(id, 8)).collect::<Vec<_>>();
    store.reset_git_tree_write_count();
    store.put_git_trees_bulk(&trees).unwrap();
    assert!(
        store.git_tree_transactions() <= 300_usize.div_ceil(64),
        "300 missing eight-entry trees took {} write transactions",
        store.git_tree_transactions()
    );

    // A tree larger than any batch budget is never split across transactions.
    store.reset_git_tree_write_count();
    store
        .put_git_trees_bulk(&[tree(1_000, 8), tree(1_001, 5_000), tree(1_002, 8)])
        .unwrap();
    assert!(store.git_tree_transactions() <= 3);

    let connection = store.connection.lock().unwrap();
    let count = |table: &str| -> i64 {
        connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    };
    assert_eq!(count("git_trees"), 303);
    assert_eq!(count("git_tree_entries"), 300 * 8 + 8 + 5_000 + 8);
}

#[test]
fn collection_reclaims_unreachable_auxiliary_metadata() {
    let (_path, store) = store("collect-auxiliary-metadata");
    let tree = HashKey::git_sha1([6; 20]);
    store
        .put_git_trees_bulk(&[GitTree {
            id: tree,
            prefix: String::new(),
            entries: vec![GitTreeEntry {
                name: "source.py".to_owned(),
                mode: 0o100644,
                target: HashKey::git_sha1([7; 20]),
            }],
        }])
        .unwrap();
    let snapshot = store
        .put_snapshot(tree, 1, HashKey::sha256(b"grammar"), 1)
        .unwrap();
    let source = store.put_source(b"source", &[]).unwrap();
    let tape = store.put_tape(source, 1, 1, 1, b"tape").unwrap();
    store
        .put_snapshot_file(snapshot, "source.py", source, Some(tape))
        .unwrap();
    let workspace = store
        .open_workspace("/collect-metadata", "repository", "owner", snapshot, 1, 100)
        .unwrap();
    store.retire_workspace(workspace).unwrap();
    for _ in 0..8 {
        store.collect(101, 100).unwrap();
    }

    let connection = store.connection.lock().unwrap();
    for table in [
        "paths",
        "strings",
        "hashes",
        "git_tree_entries",
        "git_trees",
        "completed_directory_reuse",
    ] {
        let count: i64 = connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "unreachable {table} rows remain");
    }
}
#[test]
fn collection_removes_large_unreachable_tree_in_batches() {
    let (_path, store) = store("collect-large-tree");
    let tree = HashKey::git_sha1([8; 20]);
    let entries = (0..257)
        .map(|index| GitTreeEntry {
            name: format!("file-{index}"),
            mode: 0o100644,
            target: HashKey::git_sha1([index as u8; 20]),
        })
        .collect();
    store
        .put_git_trees_bulk(&[GitTree {
            id: tree,
            prefix: String::new(),
            entries,
        }])
        .unwrap();
    let snapshot = store
        .put_snapshot(tree, 1, HashKey::sha256(b"grammar"), 1)
        .unwrap();
    let workspace = store
        .open_workspace("/large-tree", "repository", "owner", snapshot, 1, 100)
        .unwrap();
    store.retire_workspace(workspace).unwrap();

    for _ in 0..4 {
        assert!(store.collect(101, 100).is_ok());
    }
    let connection = store.connection.lock().unwrap();
    let remaining: i64 = connection
        .query_row("SELECT count(*) FROM git_trees", [], |row| row.get(0))
        .unwrap();
    assert_eq!(remaining, 0);
}

#[test]
fn collection_keeps_subtrees_reachable_from_live_snapshot_root() {
    let (_path, store) = store("collect-live-subtree");
    let root = HashKey::git_sha1([9; 20]);
    let nested = HashKey::git_sha1([10; 20]);
    store
        .put_git_trees_bulk(&[
            GitTree {
                id: root,
                prefix: String::new(),
                entries: vec![GitTreeEntry {
                    name: "nested".to_owned(),
                    mode: 0o040000,
                    target: nested,
                }],
            },
            GitTree {
                id: nested,
                prefix: "nested".to_owned(),
                entries: vec![GitTreeEntry {
                    name: "source.py".to_owned(),
                    mode: 0o100644,
                    target: HashKey::git_sha1([11; 20]),
                }],
            },
        ])
        .unwrap();
    let snapshot = store
        .put_snapshot(root, 1, HashKey::sha256(b"grammar"), 1)
        .unwrap();
    store
        .open_workspace("/live-subtree", "repository", "owner", snapshot, 1, 100)
        .unwrap();

    store.collect(0, 100).unwrap();
    let connection = store.connection.lock().unwrap();
    let remaining: i64 = connection
        .query_row("SELECT count(*) FROM git_trees", [], |row| row.get(0))
        .unwrap();
    assert_eq!(remaining, 2);
}

fn three_hundred_entry_tree(seed: u8) -> GitTree {
    GitTree {
        id: HashKey::git_sha1([seed; 20]),
        prefix: String::new(),
        entries: (0..300_u16)
            .map(|index| {
                let mut target = [seed; 20];
                target[..2].copy_from_slice(&index.to_be_bytes());
                GitTreeEntry {
                    name: format!("file-{index}"),
                    mode: 0o100644,
                    target: HashKey::git_sha1(target),
                }
            })
            .collect(),
    }
}

fn git_tree_entry_count(store: &ContentStore) -> i64 {
    store
        .connection
        .lock()
        .unwrap()
        .query_row("SELECT count(*) FROM git_tree_entries", [], |row| {
            row.get(0)
        })
        .unwrap()
}

fn schema_object_exists(connection: &Connection, kind: &str, name: &str) -> bool {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type=?1 AND name=?2)",
            [kind, name],
            |row| row.get(0),
        )
        .unwrap()
}

/// Rewrites a store into the layout the floor-16 binary wrote: index-only floor 16, both retired
/// FTS search families with their delete trigger, and Git trees without a completeness marker or
/// entry count.
fn downgrade_to_floor_sixteen(path: &std::path::Path) {
    let connection = Connection::open(path).unwrap();
    for column in ["complete", "entry_count"] {
        let present: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('git_trees') WHERE name=?1)",
                [column],
                |row| row.get(0),
            )
            .unwrap();
        if present {
            connection
                .execute_batch(&format!("ALTER TABLE git_trees DROP COLUMN {column}"))
                .unwrap();
        }
    }
    connection
        .execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS search_value_fts USING fts5(
                 token, content='', contentless_delete=1, detail=none, tokenize='ascii'
             );
             CREATE VIRTUAL TABLE IF NOT EXISTS search_text_fts USING fts5(
                 token, content='', contentless_delete=1, detail=none,
                 tokenize='unicode61 remove_diacritics 0'
             );
             DROP TRIGGER IF EXISTS delete_tape_search_index;
             CREATE TRIGGER delete_tape_search_index BEFORE DELETE ON tape_objects BEGIN
                 DELETE FROM search_kind_fts WHERE rowid=OLD.tape_id;
                 DELETE FROM search_value_fts WHERE rowid=OLD.tape_id;
                 DELETE FROM search_text_fts WHERE rowid=OLD.tape_id;
             END;
             UPDATE store_schema_revision SET revision=16 WHERE singleton=1;
             PRAGMA user_version=16;",
        )
        .unwrap();
}

#[test]
fn collection_deletes_unreachable_git_trees_whole() {
    let (_path, store) = store("collect-whole-tree");
    let tree = three_hundred_entry_tree(44);
    store
        .put_git_trees_bulk(std::slice::from_ref(&tree))
        .unwrap();
    store.collect(1, 256).unwrap();
    let remaining = git_tree_entry_count(&store);
    assert!(
        remaining == 0 || remaining == 300,
        "collection left {remaining} of 300 tree entries"
    );
    store
        .put_git_trees_bulk(std::slice::from_ref(&tree))
        .unwrap();
    assert_eq!(git_tree_entry_count(&store), 300);
}

#[test]
fn snapshots_require_a_published_root_tree() {
    let (_path, store) = store("snapshot-root");
    let root = HashKey::git_sha1([45; 20]);
    let grammar = HashKey::sha256(b"grammar");
    assert!(matches!(
        store.put_snapshot(root, 1, grammar, 1),
        Err(StoreError::Invalid(_))
    ));
    let trees: i64 = store
        .connection
        .lock()
        .unwrap()
        .query_row("SELECT count(*) FROM git_trees", [], |row| row.get(0))
        .unwrap();
    assert_eq!(trees, 0, "a rejected snapshot created a root tree row");
    published_tree(&store, root);
    assert!(store.put_snapshot(root, 1, grammar, 1).is_ok());
}

#[test]
fn upgrade_republishes_git_trees_whose_completeness_is_unproven() {
    let (path, store) = store("legacy-partial-tree");
    let tree = three_hundred_entry_tree(46);
    store
        .put_git_trees_bulk(std::slice::from_ref(&tree))
        .unwrap();
    let snapshot = store
        .put_snapshot(tree.id, 1, HashKey::sha256(b"grammar"), 1)
        .unwrap();
    store
        .open_workspace(
            "/legacy-partial-tree",
            "repo",
            "owner",
            snapshot,
            1,
            i64::MAX,
        )
        .unwrap();
    drop(store);
    downgrade_to_floor_sixteen(&path);
    // The floor-16 collector deleted unreachable tree entries 256 at a time and kept the tree row.
    Connection::open(&path)
        .unwrap()
        .execute(
            "DELETE FROM git_tree_entries WHERE (tree_id,name_string_id) IN (\
             SELECT tree_id,name_string_id FROM git_tree_entries \
             ORDER BY tree_id,name_string_id LIMIT 256)",
            [],
        )
        .unwrap();

    let store = ContentStore::open(&path).unwrap();
    let floor: i64 = store
        .connection
        .lock()
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(floor, SCHEMA_VERSION);
    store
        .put_git_trees_bulk(std::slice::from_ref(&tree))
        .unwrap();
    assert_eq!(git_tree_entry_count(&store), 300);
}

#[test]
fn upgrade_drops_retired_search_families() {
    let (path, store) = store("retired-search-families");
    drop(store);
    downgrade_to_floor_sixteen(&path);
    let store = ContentStore::open(&path).unwrap();
    let source = store.put_source(b"retired families", &[]).unwrap();
    store.put_tape(source, 1, 1, 1, b"tape").unwrap();
    for _ in 0..3 {
        store.collect(1, 100).unwrap();
    }
    let connection = store.connection.lock().unwrap();
    let tapes: i64 = connection
        .query_row("SELECT count(*) FROM tape_objects", [], |row| row.get(0))
        .unwrap();
    assert_eq!(tapes, 0);
    for family in ["search_value_fts", "search_text_fts"] {
        assert!(
            !schema_object_exists(&connection, "table", family),
            "retired search family {family} survived the upgrade"
        );
    }
}

#[test]
fn index_revisions_apply_without_raising_the_floor() {
    let (path, store) = store("index-revision");
    drop(store);
    let connection = Connection::open(&path).unwrap();
    let floor: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    connection
        .execute_batch(
            "DROP INDEX IF EXISTS snapshot_files_by_path;
             DROP INDEX IF EXISTS tape_objects_by_hash;
             DROP INDEX IF EXISTS workspaces_by_snapshot;",
        )
        .unwrap();
    connection
        .execute(
            "UPDATE store_schema_revision SET revision=?1 WHERE singleton=1",
            [STORE_REVISION - 1],
        )
        .unwrap();
    drop(connection);

    let store = ContentStore::open(&path).unwrap();
    let connection = store.connection.lock().unwrap();
    let reopened_floor: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    let revision: i64 = connection
        .query_row(
            "SELECT revision FROM store_schema_revision WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(reopened_floor, floor);
    assert_eq!(revision, STORE_REVISION);
    for index in [
        "snapshot_files_by_path",
        "tape_objects_by_hash",
        "workspaces_by_snapshot",
    ] {
        assert!(
            schema_object_exists(&connection, "index", index),
            "revision upgrade did not create {index}"
        );
    }
}

#[test]
fn no_secondary_index_repeats_a_primary_key_prefix() {
    let (_path, store) = store("redundant-indexes");
    let connection = store.connection.lock().unwrap();
    let names = |sql: &str, argument: &str| {
        connection
            .prepare(sql)
            .unwrap()
            .query_map([argument], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    let mut redundant = Vec::new();
    for table in names(
        "SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE ?1",
        "sqlite_%",
    ) {
        let primary_key = names(
            "SELECT name FROM pragma_table_info(?1) WHERE pk>0 ORDER BY pk",
            &table,
        );
        for index in names(
            "SELECT name FROM pragma_index_list(?1) WHERE origin='c' AND partial=0",
            &table,
        ) {
            let columns = names(
                "SELECT name FROM pragma_index_info(?1) ORDER BY seqno",
                &index,
            );
            if !primary_key.is_empty() && primary_key.starts_with(&columns) {
                redundant.push(index);
            }
        }
    }
    assert!(
        redundant.is_empty(),
        "indexes repeat their table's primary key: {redundant:?}"
    );
}

#[test]
fn collection_rejects_batches_above_its_statement_limit() {
    let (_path, store) = store("collect-batch-limit");
    assert!(store.collect(0, 256).is_ok());
    assert!(matches!(store.collect(0, 257), Err(StoreError::Invalid(_))));
}

#[test]
fn publication_repairs_a_complete_tree_that_lost_entries_to_another_writer() {
    let (path, store) = store("complete-tree-lost-entries");
    let tree = three_hundred_entry_tree(47);
    store
        .put_git_trees_bulk(std::slice::from_ref(&tree))
        .unwrap();
    // The entry-batched collector of an older binary that still holds the store: it deletes
    // unreachable entries 256 at a time and leaves the tree row, which says complete.
    Connection::open(&path)
        .unwrap()
        .execute(
            "DELETE FROM git_tree_entries WHERE (tree_id,name_string_id) IN (\
             SELECT tree_id,name_string_id FROM git_tree_entries \
             ORDER BY tree_id,name_string_id LIMIT 256)",
            [],
        )
        .unwrap();
    assert_eq!(git_tree_entry_count(&store), 44);
    store
        .put_git_trees_bulk(std::slice::from_ref(&tree))
        .unwrap();
    assert_eq!(
        git_tree_entry_count(&store),
        300,
        "publication trusted a complete marker over a partial entry set"
    );
}

#[test]
fn collection_bounds_the_tree_entries_one_pass_deletes() {
    let (_path, store) = store("collect-tree-entry-budget");
    let trees = (60..64).map(three_hundred_entry_tree).collect::<Vec<_>>();
    store.put_git_trees_bulk(&trees).unwrap();
    let mut remaining = git_tree_entry_count(&store);
    assert_eq!(remaining, 1_200);
    while remaining > 0 {
        store.collect(1, MAX_COLLECTION_BATCH).unwrap();
        let left = git_tree_entry_count(&store);
        let deleted = usize::try_from(remaining - left).unwrap();
        assert!(deleted > 0, "a collection pass deleted no unreachable tree");
        assert!(
            deleted <= super::snapshots::GIT_TREE_BATCH_ENTRIES,
            "one collection pass deleted {deleted} tree entries under a single write lock"
        );
        assert_eq!(deleted % 300, 0, "collection split a tree across passes");
        remaining = left;
    }
}

#[test]
fn publication_completes_a_tree_with_duplicate_entry_names() {
    let (_path, store) = store("duplicate-entry-names");
    // A legacy tree `git fsck` reports as duplicateEntries; the entry key stores one row per name.
    let mut root = three_hundred_entry_tree(48);
    root.entries.truncate(3);
    root.entries[2].name = root.entries[0].name.clone();
    let published = store.publish_snapshot(
        std::slice::from_ref(&root),
        root.id,
        1,
        HashKey::sha256(b"grammar"),
        1,
        &WorkspaceRegistration {
            root: "/duplicate-entry-names",
            repository: "repo",
            owner: "owner",
            generation: 1,
            lease_seconds: i64::MAX,
        },
    );
    assert!(
        published.is_ok(),
        "a snapshot over a tree with duplicate entry names failed: {:?}",
        published.err()
    );
    store.reset_git_tree_write_count();
    store
        .put_git_trees_bulk(std::slice::from_ref(&root))
        .unwrap();
    assert_eq!(
        store.git_tree_write_count(),
        0,
        "a stored tree with duplicate entry names never counts as complete"
    );
}
