use super::*;
use crate::content_store::tests::store;

fn manifest(id: i64, root: i64, last_time: i64, sources: &[i64]) -> Manifest {
    Manifest { id, root, last_time, grammar_version: 1, tape_format: 1, plan: vec![id as u8; 32], lane: None, policy: None, sources: sources.to_vec() }
}

fn sizes(n: i64, bytes: u64) -> Sizes {
    let mut s = Sizes::default();
    for id in 1..=n {
        s.source.insert(id, bytes);
        s.tape.insert((id, 1, 1), bytes);
    }
    s
}

#[test]
fn source_ids_round_trip_as_delta_varints() {
    let ids = vec![1, 2, 130, 20_000, 1 << 40];
    assert_eq!(decode_ids(&encode_ids(&ids)).unwrap(), ids);
    assert!(decode_ids(&[0x80]).is_err(), "a truncated varint is refused");
}

#[test]
fn analysis_scope_keeps_a_shared_manifest_until_the_last_owner_is_deleted() {
    let (_path, store) = store("analysis-scope");
    let grammar = 1;
    let format = 1;
    let now = 100;
    let plan = [1; 32];
    let nested = [2; 32];
    let root = "/virtual/repository";
    for name in ["named:review", "named:release"] {
        store.record_scoped_corpus_run(root, &[], grammar, format, plan, &[nested], None, None, now, Some(name)).unwrap();
    }
    let policy = CorpusPolicy { backstop_secs: 0, missing_secs: 0, ..CorpusPolicy::default() };
    store.keep_corpus_runs(&policy, now + 1).unwrap();
    assert_eq!(store.live_plans().unwrap(), HashSet::from([plan, nested]));

    store.delete_analysis_scope("named:review").unwrap();
    store.keep_corpus_runs(&policy, now + 1).unwrap();
    assert_eq!(store.live_plans().unwrap(), HashSet::from([plan, nested]));
    assert!(store.corpus_run_of(root).unwrap().is_some());

    store.delete_analysis_scope("named:release").unwrap();
    store.keep_corpus_runs(&policy, now + 1).unwrap();
    assert!(store.live_plans().unwrap().is_empty());
    assert!(store.corpus_run_of(root).unwrap().is_none());
}

#[test]
fn deleting_an_analysis_scope_rejects_an_active_runs_late_manifest() {
    let (_path, store) = store("analysis-scope-generation");
    let name = "named:review";
    let active = store.begin_analysis_scope(name).unwrap();
    store.delete_analysis_scope(name).unwrap();
    let root = "/virtual/repository";
    let plan = [1; 32];
    assert!(!store.record_corpus_run_retaining(root, &[], 1, 1, plan, &[], None, None, 100, None, Some(&active)).unwrap());
    assert!(store.corpus_run_of(root).unwrap().is_none());
    let next = store.begin_analysis_scope(name).unwrap();
    assert!(store.record_corpus_run_retaining(root, &[], 1, 1, plan, &[], None, None, 101, None, Some(&next)).unwrap());
}

#[test]
fn releasing_one_use_names_compacts_generations_and_never_revives_a_stale_owner() {
    let (path, store) = store("named-scope-generation-compaction");
    // Earlier binaries could advance a scope's generation independently of the acquisition sequence.
    let db = store.connection.lock().unwrap();
    db.execute("INSERT INTO analysis_scope_generation VALUES ('named:legacy',10000)", []).unwrap();
    drop(db);
    let old = store.begin_analysis_scope("named:legacy").unwrap();
    let serialized = serde_json::to_vec(&old).unwrap();
    store.delete_analysis_scope("named:legacy").unwrap();
    for n in 0..100 {
        let scope = format!("named:once-{n}");
        store.begin_analysis_scope(&scope).unwrap();
        store.delete_analysis_scope(&scope).unwrap();
    }
    let reopened = ContentStore::open(&path).unwrap();
    let db = reopened.connection.lock().unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM analysis_scope_generation WHERE scope GLOB 'named:*'", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    drop(db);
    let fresh = reopened.begin_analysis_scope("named:legacy").unwrap();
    let stale: crate::content_store::analysis_scope::AnalysisOwner = serde_json::from_slice(&serialized).unwrap();
    let publish = |owner| reopened.record_corpus_run_retaining("/repo", &[], 1, 1, [7; 32], &[], None, None, 100, None, Some(owner)).unwrap();
    assert!(!publish(&stale));
    assert!(publish(&fresh));
    reopened.delete_analysis_scope("named:legacy").unwrap();
    let again = reopened.begin_analysis_scope("named:legacy").unwrap();
    assert!(!publish(&fresh));
    assert!(publish(&again));
}

#[test]
fn replaying_a_directory_analysis_scope_release_keeps_a_later_run() {
    use crate::content_store::analysis_scope;
    let (_path, store) = store("analysis-scope-release-replay");
    let directory = std::path::Path::new("/virtual/repository");
    let name = analysis_scope::directory(directory);
    let active = store.begin_analysis_scope(&name).unwrap();
    store.delete_directory_analysis_scopes(directory, Some("stop-1")).unwrap();
    let next = store.begin_analysis_scope(&name).unwrap();
    store.delete_directory_analysis_scopes(directory, Some("stop-1")).unwrap();
    let plan = [1; 32];
    let root = "/virtual/repository";
    assert!(!store.record_corpus_run_retaining(root, &[], 1, 1, plan, &[], None, None, 100, None, Some(&active)).unwrap());
    assert!(store.record_corpus_run_retaining(root, &[], 1, 1, plan, &[], None, None, 101, None, Some(&next)).unwrap());
}

#[cfg(target_os = "linux")]
#[test]
fn directory_releases_distinguish_raw_bytes_from_the_replacement_character() {
    use crate::content_store::analysis_scope::{self, AnalysisRun, RunSource};
    use std::{ffi::OsStr, os::unix::{ffi::OsStrExt, fs::symlink}};
    let (db_path, store) = store("scope-byte-paths");
    let base = tempfile::tempdir().unwrap();
    let parent = base.path();
    let raw = parent.join(OsStr::from_bytes(b"repo-\xff"));
    let unicode = parent.join("repo-\u{fffd}");
    let raw_link = parent.join("first");
    let unicode_link = parent.join("second");
    for (dir, link) in [(&raw, &raw_link), (&unicode, &unicode_link)] {
        std::fs::create_dir(dir).unwrap();
        symlink(dir, link).unwrap();
    }
    let publish = |scope: &str, dir: &std::path::Path, id: &str| {
        let mut run = AnalysisRun::begin_at(ContentStore::open(&db_path).unwrap(), scope, id, 1).unwrap();
        run.set_directory_scope(dir);
        run.select_source(RunSource::directory(dir)).unwrap();
        assert!(store.record_corpus_run_retaining(id, &[], 1, 1, [7; 32], &[], None, None, 1, None, Some(&run.owner)).unwrap());
        run.finish(true).unwrap();
    };
    let legacy = format!("cwd:home:{}", unicode.display());
    publish(&legacy, &unicode_link, "legacy");
    publish(&analysis_scope::directory_in(&raw_link, Some("home")), &raw_link, "raw");
    publish(&analysis_scope::directory_in(&unicode_link, Some("home")), &unicode_link, "unicode");
    let survivors = || {
        let mut ids: Vec<_> = store.completed_analysis_runs().unwrap().into_iter().map(|row| row.scan_id).collect();
        ids.sort();
        ids
    };
    store.delete_directory_analysis_scopes(&raw_link, Some("2:one:home")).unwrap();
    assert_eq!(survivors(), ["legacy", "unicode"]);
    assert!(!analysis_scope::existing_directory_ownership(&db_path, &raw_link, "home").unwrap());
    assert!(analysis_scope::existing_directory_ownership(&db_path, &unicode_link, "home").unwrap());
    store.delete_directory_analysis_scopes(&unicode_link, Some("3:two:home")).unwrap();
    assert_eq!(survivors(), ["legacy"]);
    store.delete_directory_analysis_scopes(parent, Some("4:parent:home")).unwrap();
    assert!(survivors().is_empty());
}

#[test]
fn analysis_scope_stop_distinguishes_discovery_before_and_after_it() {
    use crate::content_store::analysis_scope;
    let (_path, store) = store("analysis-scope-stop-order");
    let directory = std::path::Path::new("/virtual/repository");
    let name = analysis_scope::directory(directory);
    let newer = store.begin_analysis_scope_at(&name, 200).unwrap();
    let publish = |owner| store.record_corpus_run_retaining("/virtual/repository", &[], 1, 1, [1; 32], &[], None, None, 100, None, Some(owner)).unwrap();
    assert!(publish(&newer));
    store.delete_directory_analysis_scopes(directory, Some("150:stop")).unwrap();
    let old_discovery = store.begin_analysis_scope_at(&name, 100).unwrap();
    assert!(!publish(&old_discovery));
    assert!(store.corpus_run_of("/virtual/repository").unwrap().is_some());
}

#[test]
fn analysis_scope_only_publishes_staged_memberships_on_success() {
    use crate::content_store::analysis_scope::AnalysisRun;
    let (path, store) = store("analysis-scope-publish");
    let check = ContentStore::open(&path).unwrap();
    let mut run = AnalysisRun::begin(store, "named:review", "test").unwrap();
    let root = "/virtual/repository";
    assert!(check.record_corpus_run_retaining(root, &[], 1, 1, [1; 32], &[], None, None, 100, None, Some(&run.owner)).unwrap());
    run.finish(false).unwrap();
    assert!(check.corpus_run_of(root).unwrap().is_none());
    let mut run = AnalysisRun::begin(ContentStore::open(&path).unwrap(), "named:review", "test-2").unwrap();
    assert!(check.record_corpus_run_retaining(root, &[], 1, 1, [1; 32], &[], None, None, 101, None, Some(&run.owner)).unwrap());
    run.owner.warn("injected writer failure".into());
    run.finish(true).unwrap();
    assert!(check.corpus_run_of(root).unwrap().is_none());
    let mut run = AnalysisRun::begin(ContentStore::open(&path).unwrap(), "named:review", "test-3").unwrap();
    assert!(check.record_corpus_run_retaining(root, &[], 1, 1, [1; 32], &[], None, None, 102, None, Some(&run.owner)).unwrap());
    run.finish(true).unwrap();
    assert!(check.corpus_run_of(root).unwrap().is_some());
    check.delete_analysis_scope("named:review").unwrap();
    assert!(check.corpus_run_of(root).unwrap().is_none());
}

#[test]
fn a_publication_error_rolls_back_history_and_its_ancestor_index_before_retry() {
    use crate::content_store::analysis_scope::{AnalysisRun, RunSource};
    let (path, store) = store("scope-history-rollback");
    let directory = path.parent().unwrap().join("work");
    let scope = crate::content_store::analysis_scope::directory(&directory);
    let mut run = AnalysisRun::begin(ContentStore::open(&path).unwrap(), &scope, "scan").unwrap();
    run.select_source(RunSource::Directory(directory.to_str().unwrap().to_owned())).unwrap();
    let counts = || {
        store.connection.lock().unwrap().query_row("SELECT
            (SELECT count(*) FROM analysis_scope_pending),
            (SELECT count(*) FROM analysis_history_scope WHERE scope=?1),
            (SELECT count(*) FROM analysis_scope_run WHERE scope=?1),
            EXISTS(SELECT 1 FROM analysis_history_ancestor WHERE scope=?1)", [&scope],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, bool>(3)?))).unwrap()
    };
    store.connection.lock().unwrap().execute_batch("CREATE TRIGGER fail_fixture BEFORE INSERT ON analysis_scope_run
        BEGIN SELECT RAISE(ABORT,'interrupted fixture'); END;").unwrap();
    assert!(run.finish(true).is_err());
    assert_eq!(counts(), (1, 0, 0, false));
    store.connection.lock().unwrap().execute_batch("DROP TRIGGER fail_fixture").unwrap();
    run.finish(true).unwrap();
    assert_eq!(counts(), (0, 1, 1, true));
    store.delete_analysis_scope(&scope).unwrap();
    assert_eq!(counts(), (0, 0, 0, false));
}

#[test]
fn concurrent_analysis_runs_order_snapshots_and_isolate_staging() {
    use crate::content_store::{analysis_scope::AnalysisRun, GitTree, HashKey};
    let (path, store) = store("scope-acquisitions");
    let scope = "cwd:/virtual/project";
    let begin = |scope: &str| AnalysisRun::begin_at(ContentStore::open(&path).unwrap(), scope, "client-id", 7).unwrap();
    let mut older = begin(scope);
    let mut newer = begin(scope);
    let mut abandoned = begin("named:independent");
    let tree = HashKey::git_sha1([5; 20]);
    store.put_git_trees_bulk(&[GitTree { id: tree, prefix: String::new(), entries: Vec::new() }]).unwrap();
    let snapshots = [1, 2].map(|version| store.put_snapshot(tree, version, HashKey::sha256(b"grammar"), 1).unwrap());
    for (owner, snapshot) in [(&older.owner, snapshots[0]), (&newer.owner, snapshots[1]), (&abandoned.owner, snapshots[0])] {
        store.own_analysis_snapshot(owner, "workspace", snapshot).unwrap();
    }
    abandoned.finish(false).unwrap();
    newer.finish(true).unwrap();
    older.finish(true).unwrap();
    let db = store.connection.lock().unwrap();
    let rows = db.prepare("SELECT scope,snapshot_id FROM analysis_scope_workspace_snapshot").unwrap()
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(rows, [(scope.to_owned(), snapshots[1].0)]);
}

#[test]
fn analysis_scope_rejects_finished_and_foreign_run_owners() {
    use crate::content_store::analysis_scope::AnalysisRun;
    let (path, first_store) = store("scope-closed-owner");
    let (other_path, second_store) = store("scope-foreign-owner");
    let scope = "named:review";
    let mut first = AnalysisRun::begin(ContentStore::open(&path).unwrap(), scope, "client").unwrap();
    let mut second = AnalysisRun::begin(ContentStore::open(&other_path).unwrap(), scope, "client").unwrap();
    let publish = |store: &ContentStore, owner: &_| store.record_corpus_run_retaining(
        "/root", &[], 1, 1, [1; 32], &[], None, None, 100, None, Some(owner)).unwrap();
    assert!(!publish(&second_store, &first.owner), "an owner cannot write to another store's identical staging key");
    first.finish(true).unwrap();
    assert!(!publish(&first_store, &first.owner), "an async worker cannot stage new rows after finish");
    assert!(publish(&second_store, &second.owner));
    second.finish(true).unwrap();
}

#[test]
fn named_scope_release_requires_the_registered_store_and_revokes_an_unfinished_run() {
    use crate::content_store::analysis_scope::{AnalysisRun, RunSource, release_named_scope};
    let (first_path, first) = store("scope-release-first");
    let (second_path, second) = store("scope-release-replacement");
    let mut unfinished = AnalysisRun::begin(ContentStore::open(&first_path).unwrap(), "named:test", "pending").unwrap();
    unfinished.select_source(RunSource::directory(std::path::Path::new("/repo"))).unwrap();
    let mut published = AnalysisRun::begin(ContentStore::open(&second_path).unwrap(), "named:test", "keep").unwrap();
    published.select_source(RunSource::directory(std::path::Path::new("/repo"))).unwrap();
    published.finish(true).unwrap();
    assert!(release_named_scope(&second_path, "test", &[first.identity()]).is_err());
    assert_eq!(second.completed_analysis_runs().unwrap().len(), 1);
    assert_eq!(release_named_scope(&first_path, "test", &[first.identity()]).unwrap(), (first.identity(), false));
    unfinished.finish(true).unwrap();
    assert_eq!(unfinished.publication.unwrap().as_str(), "released");
    assert!(first.completed_analysis_runs().unwrap().is_empty());
    assert_eq!(release_named_scope(&second_path, "test", &[second.identity()]).unwrap(), (second.identity(), true));
    assert_eq!(release_named_scope(&second_path, "test", &[]).unwrap(), (second.identity(), false));
}

#[test]
fn completed_analysis_runs_survive_restart_and_scope_deletion_preserves_shared_snapshots() {
    use crate::content_store::{analysis_scope::{AnalysisRun, RunSource}, GitTree, HashKey};
    let (path, store) = store("scope-run-history");
    let tree = HashKey::git_sha1([9; 20]);
    store.put_git_trees_bulk(&[GitTree { id: tree, prefix: String::new(), entries: Vec::new() }]).unwrap();
    let snapshot = store.put_snapshot(tree, 1, HashKey::sha256(b"grammar"), 1).unwrap();
    let source = RunSource::Git { repository: "repo".into(), tree: "tree".into(), commit: None };
    let begin = |scope: &str| {
        let mut run = AnalysisRun::begin(ContentStore::open(&path).unwrap(), scope, "caller-id").unwrap();
        run.select_source(source.clone()).unwrap();
        store.own_analysis_snapshot(&run.owner, "workspace", snapshot).unwrap();
        run
    };
    for scope in ["named:a", "named:a", "named:b"] {
        let mut run = begin(scope);
        let worker = run.owner.clone();
        run.finish(true).unwrap();
        assert!(!worker.warn("late warning".into()));
        run.finish(false).unwrap();
        assert_eq!(run.publication_warning(), None);
        assert_eq!(run.publication.unwrap().as_str(), "published");
    }
    begin("named:aborted").finish(false).unwrap();
    let mut warned = begin("named:warned");
    warned.owner.warn("could not retain".into());
    warned.finish(true).unwrap();
    assert!(!warned.owner.warn("later warning".into()));
    assert_eq!(warned.publication_warning().as_deref(), Some("could not retain"));
    let mut stale = begin("named:stale");
    store.delete_analysis_scope("named:stale").unwrap();
    stale.finish(true).unwrap();
    let fresh = ContentStore::open(&path).unwrap();
    let rows = fresh.completed_analysis_runs().unwrap();
    assert_eq!(rows.iter().map(|r| r.scope.as_str()).collect::<Vec<_>>(), ["named:a", "named:a", "named:b"]);
    assert!(rows.windows(2).all(|p| p[0].acquisition < p[1].acquisition));
    assert!(rows.iter().all(|r| r.scan_id == "caller-id" && r.commit.is_none() && r.snapshot == Some(snapshot.0)));
    fresh.delete_analysis_scope("named:a").unwrap();
    let rows = fresh.completed_analysis_runs().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].scope.as_str(), rows[0].snapshot), ("named:b", Some(snapshot.0)));
}

#[test]
fn adding_analysis_history_preserves_previous_manifest_owners_without_fabricating_runs() {
    let (path, store) = store("scope-history-upgrade");
    for scope in ["named:legacy", "cwd:/repo:one", "cwd:home:/repo:two", r"cwd:C:\checkout"] {
        let owner = store.begin_analysis_scope(scope).unwrap();
        store.record_corpus_run_retaining("/repo", &[], 1, 1, [1; 32], &[], None, None, 100, None, Some(&owner)).unwrap();
    }
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("DROP TRIGGER analysis_runs_end_generation; DROP TRIGGER analysis_runs_stop_directory;
        DROP TABLE analysis_run_workspace_snapshot; DROP TABLE analysis_scope_run; DROP TABLE analysis_history_scope;").unwrap();
    drop(db);
    let upgraded = ContentStore::open(&path).unwrap();
    let pending = crate::content_store::analysis_scope::AnalysisRun::begin(ContentStore::open(&path).unwrap(), "named:pending-only", "live").unwrap();
    let records = upgraded.analysis_records(true, true).unwrap();
    assert!(records.runs.is_empty());
    assert_eq!(records.scopes.iter().map(|(s,v)| (s.as_str(), v.as_str())).collect::<Vec<_>>(), [
        ("cwd:/repo:one", "/repo:one"), (r"cwd:C:\checkout", r"C:\checkout"),
        ("cwd:home:/repo:two", "/repo:two"), ("named:legacy", "legacy"),
    ]);
    drop(pending);
    upgraded.delete_analysis_scope("named:legacy").unwrap();
    assert_eq!(upgraded.analysis_records(true, false).unwrap().scopes.len(), 3);
}

#[test]
fn an_offline_scope_store_returns_read_errors_on_a_new_worker() {
    let area = tempfile::tempdir().unwrap();
    let mount = area.path().join("mounted");
    std::fs::create_dir(&mount).unwrap();
    let file = mount.join("store.sqlite");
    let store = ContentStore::open(&file).unwrap();
    crate::content_store::connection::release_all_at(&file);
    let offline = area.path().join("offline");
    std::fs::rename(&mount, &offline).unwrap();
    std::thread::scope(|threads| threads.spawn(|| {
        assert!(store.directory_release_cutoff(area.path(), "home").is_err());
        assert!(store.completed_analysis_runs().is_err());
        assert!(store.analysis_snapshot_due(1).is_err());
    }).join().unwrap());
    std::fs::rename(offline, mount).unwrap();
    assert!(store.completed_analysis_runs().unwrap().is_empty());
}

#[test]
#[cfg(unix)]
fn collection_keeps_the_database_it_verified_after_its_path_changes() {
    let area = tempfile::tempdir().unwrap();
    let mount = area.path().join("mounted");
    std::fs::create_dir(&mount).unwrap();
    let file = mount.join("store.sqlite");
    let reader = ContentStore::open(&file).unwrap();
    let lease = reader.hold_collection().unwrap();
    let id = reader.identity();
    let collector = ContentStore::open_for_collection(&file, &[id]).unwrap();
    assert!(collector.try_collect().unwrap().is_none());
    std::fs::rename(&mount, area.path().join("original")).unwrap();
    std::fs::create_dir(&mount).unwrap();
    let other = rusqlite::Connection::open(&file).unwrap();
    other.execute("CREATE TABLE unrelated(value TEXT)", []).unwrap();
    let rows = collector.completed_analysis_runs().unwrap();
    assert!(rows.is_empty());
    let sidecar = mount.join("store.sqlite.collection.lock");
    assert!(collector.try_collect().is_err());
    assert!(!sidecar.exists());
    std::fs::write(&sidecar, []).unwrap();
    assert!(matches!(collector.try_collect(), Err(crate::content_store::StoreError::Invalid(message)) if message.contains("collection lock changed")));
    assert!(ContentStore::open_for_collection(&file, &[]).is_err());
    assert_eq!(other.query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    drop(other);
    std::fs::rename(&mount, area.path().join("replacement")).unwrap();
    std::fs::rename(area.path().join("original"), &mount).unwrap();
    assert!(collector.try_collect().unwrap().is_none());
    drop(lease);
    assert!(collector.try_collect().unwrap().is_some());
}

#[test]
fn stopping_a_directory_uses_its_indexed_history_after_upgrade() {
    let (path, original) = store("indexed-stop-history");
    drop(original);
    let db = rusqlite::Connection::open(&path).unwrap();
    // Set up the previous schema, including a trigger with the old name.
    db.execute_batch("DROP TRIGGER analysis_history_index_scope; DROP TRIGGER analysis_runs_stop_directory;
        DROP TABLE analysis_history_ancestor;
        CREATE TRIGGER analysis_runs_stop_directory AFTER INSERT ON analysis_scope_release BEGIN SELECT 1; END;").unwrap();
    let insert = |scope: &str, value: &str, ancestors: &[&str]| {
        db.execute("INSERT INTO analysis_history_scope VALUES (?,?,?)", (scope, value, serde_json::to_string(ancestors).unwrap())).unwrap();
        db.execute("INSERT INTO analysis_scope_run(scope,acquisition,acquired_at,scan_id) VALUES (?,1,1,'past')", [scope]).unwrap();
    };
    insert("cwd:h:/pkg/src", "/pkg/src", &["cwd:h:/pkg", "cwd:h:/pkg/src"]);
    insert("cwd:/pkg/src", "/pkg/src", &["cwd:/pkg", "cwd:/pkg/src"]);
    insert("cwd:other:/pkg", "/pkg", &["cwd:other:/pkg"]);
    insert("cwd:h:/pkg2", "/pkg2", &["cwd:h:/pkg2"]);
    db.execute("INSERT INTO analysis_scope_run(scope,acquisition,acquired_at,scan_id) VALUES ('cwd:h:/pkg/src',2,11,'new')", []).unwrap();
    drop(db);
    let upgraded = ContentStore::open(&path).unwrap();
    let db = upgraded.connection.lock().unwrap();
    let stop = || {
        db.execute_batch("SAVEPOINT history_delivery").unwrap();
        let mut statement = db.prepare("INSERT INTO analysis_scope_release(id,scope,before) VALUES ('10:test:h','cwd:h:/pkg',10)").unwrap();
        statement.execute([]).unwrap();
        let steps = statement.get_status(rusqlite::StatementStatus::VmStep);
        let survivors: Vec<String> = db.prepare("SELECT scope FROM analysis_scope_run WHERE scope GLOB 'cwd:*' ORDER BY scope").unwrap()
            .query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        assert_eq!(survivors, ["cwd:h:/pkg/src", "cwd:h:/pkg2", "cwd:other:/pkg"]);
        db.execute_batch("ROLLBACK TO history_delivery; RELEASE history_delivery").unwrap();
        steps
    };
    let baseline = stop();
    db.execute_batch("WITH RECURSIVE names(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM names WHERE n<1000)
        INSERT INTO analysis_history_scope SELECT 'named:'||n,CAST(n AS TEXT),'[]' FROM names;
        INSERT INTO analysis_scope_run(scope,acquisition,acquired_at,scan_id)
        SELECT scope,1,1,'other' FROM analysis_history_scope WHERE scope GLOB 'named:*';").unwrap();
    let populated = stop();
    assert!(populated <= baseline + 20, "stop grew from {baseline} to {populated} SQLite instructions");
}

#[test]
fn numbered_directory_stops_keep_one_cutoff_per_directory() {
    use crate::content_store::analysis_scope;
    let (_path, store) = store("scope-high-water");
    let parent = std::path::Path::new("/virtual/project");
    let child = parent.join("pkg");
    for (dir, id) in [(parent, "10:a"), (&*child, "30:b"), (parent, "20:c"), (parent, "10:a")] {
        store.delete_directory_analysis_scopes(dir, Some(id)).unwrap();
    }
    for (dir, generation, expected) in [(parent, 19, false), (parent, 21, true), (&*child, 21, false), (&*child, 31, true)] {
        let owner = store.begin_analysis_scope_at(&analysis_scope::directory(dir), generation).unwrap();
        let db = store.connection.lock().unwrap();
        assert_eq!(owner.current(&db).unwrap(), expected);
    }
    let db = store.connection.lock().unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM analysis_scope_release", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
}

#[test]
fn a_numbered_home_stop_revokes_older_unqualified_directory_writers_once() {
    use crate::content_store::analysis_scope::{self, AnalysisRun, RunSource};
    let (path, store) = store("scope-legacy-pending-stop");
    let parent = std::path::Path::new("/virtual/project");
    let child = parent.join("pkg");
    let begin = |home, generation, id| {
        let scope = analysis_scope::directory_in(&child, home);
        let mut run = AnalysisRun::begin_at(ContentStore::open(&path).unwrap(), &scope, id, generation).unwrap();
        run.select_source(RunSource::directory(&child)).unwrap();
        assert!(store.record_corpus_run_retaining(id, &[], 1, 1, [7; 32], &[], None, None, 100, None, Some(&run.owner)).unwrap());
        run
    };
    begin(None, 0, "published").finish(true).unwrap();
    let mut old = begin(None, 0, "old");
    let mut newer = begin(None, 11, "higher-generation");
    let mut other = begin(Some("other"), 5, "other-home");
    let mut qualified = begin(Some("home"), 5, "qualified");
    store.delete_directory_analysis_scopes(parent, Some("10:stop:home")).unwrap();
    let mut fresh = begin(None, 0, "after-delivery");
    store.delete_directory_analysis_scopes(parent, Some("10:stop:home")).unwrap();
    for r in [&mut old, &mut newer, &mut other, &mut qualified, &mut fresh] { r.finish(true).unwrap(); }
    let mut scans: Vec<_> = store.completed_analysis_runs().unwrap().into_iter().map(|r| r.scan_id).collect();
    scans.sort();
    assert_eq!(scans, ["after-delivery", "higher-generation", "other-home"]);
    for id in ["old", "qualified", "published"] {
        assert!(store.corpus_run_of(id).unwrap().is_none(), "{id} still has a manifest");
    }
    for id in &scans {
        assert!(store.corpus_run_of(id).unwrap().is_some(), "{id} lost its manifest");
    }
}

#[test]
fn analysis_scope_keeps_each_directory_policy_sets_latest_manifest_under_pressure() {
    let (_path, store) = store("analysis-scope-pressure");
    let root = "/virtual/repository";
    let old = [1; 32];
    let latest = [2; 32];
    let other = [3; 32];
    let independent = [4; 32];
    for (scope, plan, set) in [("cwd:/one", old, [0; 32]), ("cwd:/one", latest, [0; 32]), ("cwd:/two", other, [0; 32]), ("cwd:/one", independent, [9; 32])] {
        let source = store.put_source(&plan, &[]).unwrap();
        store.record_scoped_corpus_run(root, &[source], 1, 1, plan, &[], Some(plan), Some(set), 100, Some(scope)).unwrap();
    }
    let policy = CorpusPolicy { protected_runs: 0, low_water: 0, high_water: 0, backstop_secs: 0, missing_secs: 0, ..CorpusPolicy::default() };
    store.keep_corpus_runs(&policy, 101).unwrap();
    assert_eq!(store.live_plans().unwrap(), HashSet::from([latest, other, independent]));
}

#[test]
fn directory_manifest_protection_follows_its_own_successful_acquisitions() {
    use crate::content_store::analysis_scope::AnalysisRun;
    let (path, store) = store("scope-manifest-order");
    let begin = |scope: &str| AnalysisRun::begin_at(ContentStore::open(&path).unwrap(), scope, "scan", 7).unwrap();
    let record = |run: &AnalysisRun, plan: [u8; 32]| {
        let source = store.put_source(&plan, &[]).unwrap();
        store.record_corpus_run_retaining("/repo", &[source], 1, 1, plan, &[], Some(plan), Some([0; 32]),
            100, None, Some(&run.owner)).unwrap();
    };
    let scope = "cwd:/repo";
    let old = [1; 32];
    let latest = [2; 32];
    let mut older = begin(scope);
    let mut newer = begin(scope);
    record(&newer, latest);
    newer.finish(true).unwrap();
    record(&older, old);
    older.finish(true).unwrap();
    let mut aborted = begin(scope);
    record(&aborted, old);
    aborted.finish(false).unwrap();
    let mut other = begin("named:other");
    record(&other, old);
    other.finish(true).unwrap();
    store.delete_analysis_scope("named:other").unwrap();
    let policy = CorpusPolicy { protected_runs: 0, low_water: 0, high_water: 0, backstop_secs: 0, missing_secs: 0, ..CorpusPolicy::default() };
    store.keep_corpus_runs(&policy, 101).unwrap();
    assert_eq!(store.live_plans().unwrap(), HashSet::from([latest]));
}

#[test]
fn migrating_directory_membership_freezes_the_existing_manifest_order() {
    let (path, store) = store("scope-manifest-upgrade");
    for plan in [[1; 32], [2; 32]] {
        store.record_scoped_corpus_run("/repo", &[], 1, 1, plan, &[], None, Some([0; 32]), 100, Some("cwd:/repo")).unwrap();
    }
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TEMP TABLE old_rows AS SELECT scope,manifest_id,acquired_at FROM analysis_scope_manifest;
        DROP TABLE analysis_scope_manifest;
        CREATE TABLE analysis_scope_manifest (
            scope TEXT NOT NULL, manifest_id INTEGER NOT NULL REFERENCES corpus_manifests(manifest_id) ON DELETE CASCADE,
            acquired_at INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(scope,manifest_id)
        ) STRICT, WITHOUT ROWID;
        INSERT INTO analysis_scope_manifest SELECT * FROM old_rows").unwrap();
    drop(db);
    let store = ContentStore::open(&path).unwrap();
    let db = store.connection.lock().unwrap();
    let rows = db.prepare("SELECT a.acquisition,a.recorded_run,m.last_run FROM analysis_scope_manifest a
        JOIN corpus_manifests m USING(manifest_id) ORDER BY a.recorded_run").unwrap()
        .query_map([], |row| Ok((row.get::<_, i64>(0)?,row.get::<_, i64>(1)?,row.get::<_, i64>(2)?))).unwrap()
        .collect::<rusqlite::Result<Vec<_>>>().unwrap();
    assert_eq!(rows, [(0, 1, 1), (0, 2, 2)]);
}

#[test]
fn analysis_scope_cache_keys_distinguish_parser_and_tape_versions() {
    let (_path, store) = store("analysis-scope-tape-version");
    let bytes = b"x = 1\n";
    let source = store.put_source(bytes, &[]).unwrap();
    let original = store.put_tape(source, 1, 2, 3, b"old tape").unwrap();
    assert_eq!(store.tape_by_source(source, 1, 2, 3).unwrap(), Some(original));
    assert_eq!(store.tape_by_source(source, 1, 4, 3).unwrap(), None);
    assert_eq!(store.tape_by_source(source, 1, 2, 4).unwrap(), None);
    let new = store.put_tape(source, 1, 4, 3, b"new tape").unwrap();
    assert_ne!(original, new);
}

#[test]
fn the_last_runs_are_protected_and_history_is_kept_to_the_low_water_mark() {
    // Root 1 churns every file each run: sources 1-2, 3-4, 5-6, 7-8, 9-10 (newest first below).
    let ms: Vec<Manifest> = (0..5).map(|k| manifest(10 - k, 1, 100, &[2 * (5 - k) - 1, 2 * (5 - k)])).collect();
    let s = sizes(10, 10);
    // Each manifest holds 40 bytes alone. Two protected; history of three is 120 bytes.
    let policy = CorpusPolicy { protected_runs: 2, high_water: 1000, low_water: 50, backstop_secs: 1000, ..CorpusPolicy::default() };
    let all = decide(&ms, &s, &policy, 100);
    assert_eq!(all.kept, vec![10, 9, 8, 7, 6], "under the high-water mark nothing is evicted");
    assert_eq!((all.protected, all.history), (80, 120));
    let policy = CorpusPolicy { high_water: 100, ..policy };
    let capped = decide(&ms, &s, &policy, 100);
    assert_eq!(capped.kept, vec![10, 9, 8], "above it, history is kept newest first to the low-water mark");
    assert_eq!(capped.history, 40);
}

#[test]
fn a_manifest_holding_nothing_of_its_own_goes_and_an_idle_root_keeps_nothing() {
    let ms = vec![manifest(3, 1, 100, &[1, 2]), manifest(2, 1, 100, &[1, 2, 3]), manifest(1, 1, 100, &[1, 2])];
    let s = sizes(3, 10);
    let policy = CorpusPolicy { protected_runs: 1, high_water: 1000, low_water: 500, backstop_secs: 50, ..CorpusPolicy::default() };
    assert_eq!(decide(&ms, &s, &policy, 100).kept, vec![3, 2], "manifest 1's sources are all in newer kept ones");
    // Root 2 last ran at time 10: past the backstop at 100, its protected run goes too.
    let ms = vec![manifest(2, 1, 100, &[1]), manifest(1, 2, 10, &[2])];
    assert_eq!(decide(&ms, &s, &policy, 100).kept, vec![2]);
}

/// Content compression cannot shrink: a different pseudo-random byte string per (file, version).
fn noise(seed: u64, n: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            b'a' + (x % 26) as u8
        })
        .collect()
}

#[test]
fn churning_every_file_keeps_the_history_under_the_high_water_mark() {
    let (_path, store) = store("corpus-churn");
    let policy = CorpusPolicy { protected_runs: 2, high_water: 64 << 10, low_water: 48 << 10, backstop_secs: 86_400, ..CorpusPolicy::default() };
    let (files, size) = (8, 2048);
    let mut runs: Vec<Vec<SourceId>> = Vec::new();
    for version in 0..20u64 {
        let mut sources = Vec::new();
        for file in 0..files {
            let source = store.put_source(&noise(version * 1000 + file, size), &[]).unwrap();
            store.put_tape(source, 1, 7, 9, &noise(version * 1000 + file + 500, size)).unwrap();
            sources.push(source);
        }
        assert!(store.record_corpus_run("/repo", &sources, 7, 9, [version as u8; 32], 1_000 + version as i64).unwrap());
        // A warm run of the same files and plan only moves the manifest.
        assert!(!store.record_corpus_run("/repo", &sources, 7, 9, [version as u8; 32], 1_000 + version as i64).unwrap());
        runs.push(sources);
        // Passes until two in a row find nothing (a pass keeps rows created in its own generation).
        let mut idle = 0;
        while idle < 2 {
            let pass = store.collect_corpus_pass(&policy, 1_000 + version as i64).unwrap();
            if let Some(kept) = &pass.kept {
                assert!(kept.history_bytes <= policy.high_water, "version {version}: history {} over the high-water mark", kept.history_bytes);
            }
            idle = if pass.did_work() { 0 } else { idle + 1 };
        }
        // The last two runs never miss.
        for sources in runs.iter().rev().take(2) {
            for &s in sources {
                assert!(store.tape_by_source(s, 1, 7, 9).unwrap().is_some(), "version {version}: a protected run's tape was collected");
            }
        }
    }
    // The oldest versions are gone: a version holds 8 files of about 4 KB, and the history is under 64 KB.
    let first = &runs[0];
    assert!(first.iter().all(|&s| store.tape_by_source(s, 1, 7, 9).unwrap().is_none()), "the first version's tapes were never collected");
    let kept: i64 = {
        let connection = store.connection.lock().unwrap();
        connection.query_row("SELECT count(*) FROM corpus_manifests", [], |r| r.get(0)).unwrap()
    };
    assert!((3..20).contains(&kept), "{kept} manifests kept of 20");
    // The pages the collected versions held went back to the file system.
    let free: i64 = store.connection.lock().unwrap().query_row("PRAGMA freelist_count", [], |r| r.get(0)).unwrap();
    assert!(free < 64, "{free} free pages left in the file");
}

#[test]
fn a_new_store_vacuums_incrementally_and_an_old_one_converts() {
    let (path, store) = store("corpus-vacuum");
    let mode = |store: &ContentStore| -> i64 { store.connection.lock().unwrap().query_row("PRAGMA auto_vacuum", [], |r| r.get(0)).unwrap() };
    assert_eq!(mode(&store), 2, "a new store returns freed pages (auto_vacuum set before WAL)");
    assert!(!store.convert_to_incremental_vacuum().unwrap());
    drop(store);
    // A store made the old way: WAL first, so auto_vacuum never took.
    let old = path.with_extension("old.sqlite");
    {
        let c = rusqlite::Connection::open(&old).unwrap();
        let _: String = c.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0)).unwrap();
        c.execute_batch("BEGIN; PRAGMA auto_vacuum=INCREMENTAL; CREATE TABLE early(x); COMMIT;").unwrap();
    }
    let store = ContentStore::open(&old).unwrap();
    assert_eq!(mode(&store), 0);
    assert!(store.convert_to_incremental_vacuum().unwrap());
    assert_eq!(mode(&store), 2);
}

#[test]
fn the_stat_index_trusts_old_unchanged_stats_of_stored_sources() {
    let (_path, store) = store("corpus-stats");
    let a = store.put_source(b"a = 1\n", &[]).unwrap();
    let b = store.put_source(b"b = 2\n", &[]).unwrap();
    let at = |mtime_ns| FileStat { size: 6, mtime_ns, inode: 7 };
    let now = 100 * RACY_NS;
    // `a` last changed long before the recording; `b` within the racy window.
    store.record_corpus_stats("/repo", &[("a.py".into(), at(now - 3 * RACY_NS), a), ("b.py".into(), at(now - RACY_NS / 2), b)], now).unwrap();
    let index = store.corpus_stats("/repo").unwrap();
    assert_eq!(index.source("a.py", at(now - 3 * RACY_NS)), Some(a));
    assert_eq!(index.source("a.py", at(now)), None, "a changed stat is read again");
    assert_eq!(index.source("a.py", FileStat { inode: 8, ..at(now - 3 * RACY_NS) }), None, "so is a replaced file");
    assert_eq!(index.source("b.py", at(now - RACY_NS / 2)), None, "an entry in the racy window is not trusted");
    assert!(store.corpus_stats("/other").unwrap().is_empty(), "stats are per root");
    // A later recording of `b` past the window is trusted, and replaces the first.
    store.record_corpus_stats("/repo", &[("b.py".into(), at(now - RACY_NS / 2), b)], now + 2 * RACY_NS).unwrap();
    assert_eq!(store.corpus_stats("/repo").unwrap().source("b.py", at(now - RACY_NS / 2)), Some(b));
}

/// O95: a source looked up, then collected before its tape is stored, fails the tape's foreign key; a source and
/// tape stored in one transaction cannot be split by a pass.
#[test]
fn a_source_collected_before_its_tape_is_stored_again_with_it() {
    let (path, store) = store("corpus-source-tape");
    let collector = ContentStore::open(&path).unwrap();
    let bytes = noise(1, 512);
    let source = store.put_source(&bytes, &[]).unwrap();
    // Two passes: the first advances the generation, the second collects the source no tape holds.
    for _ in 0..2 {
        collector.collect(1_000, MAX_COLLECTION_BATCH).unwrap();
    }
    assert!(store.put_tape(source, 1, 7, 9, &noise(2, 512)).is_err(), "the tape of a collected source has no source");
    let (now, tape) = store.put_source_and_tape(&bytes, crate::content_store::HashKey::sha256(&bytes), 1, 7, 9, &noise(2, 512)).unwrap();
    assert_eq!(store.tape_by_source(now, 1, 7, 9).unwrap(), Some(tape));
}

/// O95: writers and a collector on one store, at once. Each writer stores sources with their tapes and records a
/// run of a plan of its own; no write fails, and a plan a root's newest run names stays live (a decision is read
/// and applied in one transaction, so it never marks dead a plan recorded after it looked).
#[test]
fn parallel_writers_and_a_collector_share_one_store() {
    let (path, store) = store("corpus-parallel");
    drop(store);
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let collector = {
        let (path, stop) = (path.to_path_buf(), stop.clone());
        std::thread::spawn(move || {
            let store = ContentStore::open(&path).unwrap();
            let policy = CorpusPolicy::default();
            let mut passes = 0;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                store.collect_corpus_pass(&policy, 1_000).unwrap();
                passes += 1;
            }
            passes
        })
    };
    let writers: Vec<_> = (0..3u64)
        .map(|w| {
            let path = path.to_path_buf();
            std::thread::spawn(move || {
                let store = ContentStore::open(&path).unwrap();
                for i in 0..40u64 {
                    let seed = w * 1_000 + i;
                    let bytes = noise(seed, 256);
                    // A lookup first, as a run does; the source may be collected before its tape is stored.
                    let looked = store.put_source(&bytes, &[]).unwrap();
                    let (source, _) = store.put_source_and_tape(&bytes, crate::content_store::HashKey::sha256(&bytes), 1, 7, 9, &noise(seed + 500, 256)).unwrap();
                    let _ = looked;
                    let plan = [(seed % 251) as u8 + 1; 32];
                    store.record_corpus_run(&format!("/w{w}/r{i}"), &[source], 7, 9, plan, 1_000).unwrap();
                    let live: i64 = store.connection.lock().unwrap().query_row("SELECT live FROM corpus_plans WHERE plan_digest=?1", [&plan[..]], |r| r.get(0)).unwrap();
                    assert_eq!(live, 1, "writer {w}, run {i}: the plan of a root's newest run was marked dead");
                }
            })
        })
        .collect();
    for w in writers {
        w.join().unwrap();
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(collector.join().unwrap() > 0);
}

/// O95: a plan an evicted manifest left dead is held by a run that writes its rows before recording its manifest;
/// held, no decision marks it dead, and released, the next decision does.
#[test]
fn a_held_plan_stays_live_until_its_run_releases_it() {
    let (_path, store) = store("corpus-held-plan");
    let policy = CorpusPolicy { protected_runs: 1, high_water: 0, low_water: 0, backstop_secs: 86_400, ..CorpusPolicy::default() };
    let live = |plan: [u8; 32]| -> Option<i64> {
        store.connection.lock().unwrap().query_row("SELECT live FROM corpus_plans WHERE plan_digest=?1", [&plan[..]], |r| r.get(0)).optional().unwrap()
    };
    let decide = |seed: u64, plan: [u8; 32]| {
        let source = store.put_source(&noise(seed, 256), &[]).unwrap();
        store.put_tape(source, 1, 7, 9, &noise(seed + 500, 256)).unwrap();
        assert!(store.record_corpus_run("/repo", &[source], 7, 9, plan, 1_000).unwrap());
        store.keep_corpus_runs(&policy, 1_000).unwrap();
    };
    let old = [1u8; 32];
    decide(1, old);
    decide(2, [2u8; 32]);
    assert_eq!(live(old), Some(0), "the evicted manifest's plan is dead");
    store.hold_corpus_plan(old, "corpus-run:test", 2_000).unwrap();
    assert_eq!(live(old), Some(1), "holding a plan makes it live");
    decide(3, [3u8; 32]);
    assert_eq!(live(old), Some(1), "a decision keeps a held plan live");
    store.release_pins("corpus-run:test").unwrap();
    decide(4, [4u8; 32]);
    assert_eq!(live(old), Some(0), "released, it is dead again");
}

/// Decision 0306 step 4: a running run's collection lease alone keeps what it reads and writes from every collector
/// (another handle's, as another process's): no hold, no pin. Once its manifest names its sources and plan and the
/// lease goes, collection keeps what the manifest names and takes the rest.
#[test]
fn a_runs_lease_keeps_its_sources_and_plan_from_collection_without_holds() {
    let (path, store) = store("corpus-run-lease");
    let collector = ContentStore::open(&path).unwrap();
    let policy = CorpusPolicy { protected_runs: 1, high_water: 0, low_water: 0, backstop_secs: 86_400, decide_every_secs: 1, ..CorpusPolicy::default() };
    let passes = |now: i64| {
        for _ in 0..3 {
            collector.collect(now, MAX_COLLECTION_BATCH).unwrap();
            collector.collect_corpus_pass(&policy, now).unwrap();
        }
    };
    let plan = [41u8; 32];
    let rows = || -> i64 { store.connection.lock().unwrap().query_row("SELECT count(*) FROM facts_results WHERE plan_digest=?1", [&plan[..]], |r| r.get(0)).unwrap() };
    // An earlier run's manifest, under another plan: what a collector decides from.
    let earlier = store.put_source(&noise(20, 256), &[]).unwrap();
    assert!(store.record_corpus_run_naming("/repo", &[earlier], 7, 9, [40u8; 32], &[], 1_000).unwrap());
    let lease = store.hold_collection().unwrap();
    let (named, unnamed) = (noise(21, 256), noise(22, 256));
    let named_id = store.put_source(&named, &[]).unwrap();
    store.put_source(&unnamed, &[]).unwrap();
    let set = crate::content_store::FactsPlanSet::new(vec![plan]);
    let result = crate::content_store::FactsResult { plan, path_dependent: true, entry: vec![1, 2, 3] };
    store.put_facts_memo(crate::content_store::FactsMemoKey { source: named_id, fact_schema: live_fact_schema(150), path: "a.py" }, &set, &[result], &[]).unwrap();
    passes(2_000);
    let found = |bytes: &[u8]| store.source_by_hash(crate::content_store::HashKey::sha256(bytes)).unwrap();
    assert!(found(&named).is_some() && found(&unnamed).is_some(), "the run's sources, neither held nor named, are kept while it runs");
    assert_eq!(rows(), 1, "and its plan's rows");
    // The run's record: its manifest names the one source and the plan; then the lease goes.
    assert!(store.record_corpus_run_naming("/repo", &[named_id], 7, 9, plan, &[], 3_000).unwrap());
    drop(lease);
    passes(4_000);
    assert_eq!(found(&named), Some(named_id), "named by the manifest, kept");
    assert_eq!(rows(), 1, "the plan's rows, named, kept");
    assert_eq!(found(&unnamed), None, "named by nothing, collected once the run is done");
}

/// Old memo rows of every writer go, while their source lives on: a prepared program's plans a later build replaced
/// (named by the manifests, `record_corpus_run_naming`), a held plan whose hold lapsed (the slice's memo), and rows
/// of the corpus memo's retired schemas. A plan still named, a hold still running and other schemas' rows stay.
#[test]
fn old_memo_rows_are_collected_while_their_source_lives() {
    let (_path, store) = store("corpus-old-memo");
    let policy = CorpusPolicy { protected_runs: 1, high_water: 0, low_water: 0, backstop_secs: 86_400, decide_every_secs: 1 << 40, ..CorpusPolicy::default() };
    let source = store.put_source(&noise(1, 256), &[]).unwrap();
    store.put_tape(source, 1, 7, 9, &noise(2, 256)).unwrap();
    let put = |schema: u32, path: &str, plan: [u8; 32]| {
        let set = crate::content_store::FactsPlanSet::new(vec![plan]);
        // The corpus memo's rows are under their path; the others read none.
        let result = crate::content_store::FactsResult { plan, path_dependent: schema == 2, entry: vec![1, 2, 3] };
        store.put_facts_memo(crate::content_store::FactsMemoKey { source, fact_schema: schema, path }, &set, &[result], &[]).unwrap();
    };
    let rows = |plan: [u8; 32]| -> i64 {
        store.connection.lock().unwrap().query_row("SELECT count(*) FROM facts_results WHERE plan_digest=?1", [&plan[..]], |r| r.get(0)).unwrap()
    };
    let passes = |now: i64| {
        for _ in 0..4 {
            if !store.corpus_pass_wanted(&policy, now).unwrap() {
                break;
            }
            store.collect_corpus_pass(&policy, now).unwrap();
        }
    };
    // Two builds' runs, each naming its memo plan and its prepared program's plan, over the same source.
    let (old_pointer, new_pointer) = ([11u8; 32], [12u8; 32]);
    put(101, "rerail/prepared", old_pointer);
    assert!(store.record_corpus_run_naming("/repo", &[source], 7, 9, [1u8; 32], &[old_pointer], 1_000).unwrap());
    put(101, "rerail/prepared", new_pointer);
    assert!(store.record_corpus_run_naming("/repo", &[source], 7, 9, [2u8; 32], &[new_pointer], 1_000).unwrap());
    // A writer that holds its plans: one hold lapses at 1,500, one runs to 9,000.
    let (lapsed, running) = ([21u8; 32], [22u8; 32]);
    store.hold_corpus_plan(lapsed, "slice-memo", 1_500).unwrap();
    store.hold_corpus_plan(running, "slice-memo", 9_000).unwrap();
    put(1, "a.py", lapsed);
    put(1, "a.py", running);
    // Rows of a retired corpus memo schema, and of the slice's schema under a plan nothing ever recorded.
    let (retired, unrecorded) = ([31u8; 32], [32u8; 32]);
    put(2, "a.py", retired);
    put(1, "b.py", unrecorded);
    passes(1_000);
    assert_eq!((rows(old_pointer), rows(new_pointer)), (0, 1), "the replaced build's program plan goes; the kept one stays");
    assert_eq!(rows(retired), 0, "a retired schema's rows are swept");
    assert_eq!((rows(lapsed), rows(running), rows(unrecorded)), (1, 1, 1), "holds still running at 1,000");
    passes(2_000);
    assert_eq!((rows(lapsed), rows(running), rows(unrecorded)), (0, 1, 1), "the lapsed hold's rows go, with no new run to prompt it");
    assert!(store.get_source(source).is_ok(), "the source lives on");
}

/// Decision 0117: a kept manifest past a root's protected runs keeps its sources, not its plan (a plan's memo serves
/// one engine build only); and the hold of a run whose process is gone (a killed run never releases it) is released by
/// the next collection pass, which then collects every dead plan, not one a pass.
#[test]
fn only_protected_runs_keep_plans_and_a_dead_process_holds_nothing() {
    let (_path, store) = store("corpus-protected-plans");
    let policy = CorpusPolicy { protected_runs: 1, high_water: u64::MAX / 4, low_water: u64::MAX / 4, backstop_secs: 86_400, ..CorpusPolicy::default() };
    let live = |plan: [u8; 32]| -> Option<i64> {
        store.connection.lock().unwrap().query_row("SELECT live FROM corpus_plans WHERE plan_digest=?1", [&plan[..]], |r| r.get(0)).optional().unwrap()
    };
    let plans = [[41u8; 32], [42u8; 32], [43u8; 32]];
    for (i, plan) in plans.iter().enumerate() {
        let source = store.put_source(&noise(60 + i as u64, 256), &[]).unwrap();
        store.put_tape(source, 1, 7, 9, &noise(560 + i as u64, 256)).unwrap();
        assert!(store.record_corpus_run("/repo", &[source], 7, 9, *plan, 1_000 + i as i64).unwrap());
        store.keep_corpus_runs(&policy, 1_000 + i as i64).unwrap();
    }
    let manifests: i64 = store.connection.lock().unwrap().query_row("SELECT count(*) FROM corpus_manifests", [], |r| r.get(0)).unwrap();
    assert_eq!(manifests, 3, "under the high-water mark every manifest is kept");
    assert_eq!(plans.map(live), [Some(0), Some(0), Some(1)], "only the protected run's plan is live");
    // A process that has exited: its hold is an orphan.
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    store.hold_corpus_plan(plans[0], &format!("corpus-run:{pid}:0:0"), 9_000).unwrap();
    store.hold_corpus_plan(plans[1], &format!("corpus-run:{}:0:0", std::process::id()), 9_000).unwrap();
    assert_eq!(live(plans[0]), Some(1), "held, the plan is live");
    for _ in 0..3 {
        store.collect_corpus_pass(&policy, 1_010).unwrap();
    }
    assert_eq!(live(plans[0]), None, "the dead process's hold went, and its plan with it");
    assert_eq!(live(plans[1]), Some(1), "a running process's hold stays");
    assert_eq!(live(plans[2]), Some(1));
}

/// Programs run one at a time over a root, each its own lane (a policy set's, one process each): each keeps its plan
/// live, however many run between; a lane's replaced build loses its plan as before; and past `protected_lanes`, the
/// root's oldest lanes lose theirs.
#[test]
fn each_lane_keeps_its_last_runs_plans() {
    let (_path, store) = store("corpus-lanes");
    let policy = CorpusPolicy { protected_runs: 1, protected_lanes: 6, high_water: 0, low_water: 0, backstop_secs: 86_400, ..CorpusPolicy::default() };
    let live = |plan: [u8; 32]| -> Option<i64> {
        store.connection.lock().unwrap().query_row("SELECT live FROM corpus_plans WHERE plan_digest=?1", [&plan[..]], |r| r.get(0)).optional().unwrap()
    };
    let source = store.put_source(&noise(70, 256), &[]).unwrap();
    store.put_tape(source, 1, 7, 9, &noise(570, 256)).unwrap();
    let mut now = 1_000;
    let mut run = |plan: [u8; 32], lane: u8| {
        now += 1;
        store.record_corpus_run_lane("/repo", &[source], 7, 9, plan, &[], Some([lane; 32]), now).unwrap();
        store.keep_corpus_runs(&policy, now).unwrap();
    };
    // Five programs, then the five again: every plan live.
    let plans: Vec<[u8; 32]> = (0..5u8).map(|i| [100 + i; 32]).collect();
    for _ in 0..2 {
        for (i, plan) in plans.iter().enumerate() {
            run(*plan, i as u8);
        }
    }
    assert_eq!(plans.iter().map(|p| live(*p)).collect::<Vec<_>>(), vec![Some(1); 5], "each lane keeps its plan");
    // Lane 0 under a new build: its old plan dies, the others stay.
    let rebuilt = [110u8; 32];
    run(rebuilt, 0);
    assert_eq!((live(plans[0]), live(rebuilt)), (Some(0), Some(1)), "a replaced build's plan dies");
    assert!(plans[1..].iter().all(|p| live(*p) == Some(1)));
    // Three new lanes: the root has eight, and its two oldest (lanes 1 and 2) are past the newest six.
    for lane in 20..23u8 {
        run([lane + 100; 32], lane);
    }
    assert_eq!(plans[1..].iter().map(|p| live(*p)).collect::<Vec<_>>(), vec![Some(0), Some(0), Some(1), Some(1)]);
}

/// A lane names its policy set (where its programs come from, and the run's options): an edit of the set's programs
/// is a new lane of the same set, and supersedes the older, which keeps no protection (its plan dies, and the frames
/// it alone held go with the next sweep, decision 0227). Another set's lane over the root keeps its own, and so does
/// a run naming no set, its lane its own set (`record_run`). A run back on the older programs (an edit undone) supersedes the newer.
/// Q19: each edit had left its predecessor protected, up to `protected_lanes` of them, and the cross part's space,
/// one per root, grew with every edit.
#[test]
fn a_policy_sets_newer_lane_supersedes_its_older() {
    let (_path, store) = store("corpus-policy-sets");
    // Decisions only when one is worth it (`corpus_decision_worth`): an edit must make one.
    let policy = CorpusPolicy { protected_runs: 1, protected_lanes: 6, high_water: 0, low_water: 0, backstop_secs: 86_400, decide_every_runs: 1_000, decide_every_secs: 86_400, ..CorpusPolicy::default() };
    // A dead plan's row goes in the same pass (its memo rows first): not live either way.
    let live = |plan: [u8; 32]| -> bool {
        store.connection.lock().unwrap().query_row("SELECT live FROM corpus_plans WHERE plan_digest=?1", [&plan[..]], |r| r.get::<_, i64>(0)).optional().unwrap() == Some(1)
    };
    let source = store.put_source(&noise(71, 256), &[]).unwrap();
    store.put_tape(source, 1, 7, 9, &noise(571, 256)).unwrap();
    let mut now = 1_000;
    let mut run = |plan: [u8; 32], lane: u8, set: Option<u8>| {
        now += 1;
        store.record_corpus_run_policy("/repo", &[source], 7, 9, plan, &[], Some([lane; 32]), set.map(|s| [s; 32]), now).unwrap();
        store.collect_corpus_pass(&policy, now).unwrap();
    };
    let (a, b, none) = ([101u8; 32], [102u8; 32], [103u8; 32]);
    run(a, 1, Some(1));
    run(b, 2, Some(2));
    run(none, 3, Some(3));
    assert_eq!([a, b, none].map(live), [true; 3], "three lanes, each protected");
    // Set 1's programs edited: lane 4.
    let edited = [104u8; 32];
    run(edited, 4, Some(1));
    assert_eq!([a, edited, b, none].map(live), [false, true, true, true], "the edited set's older lane superseded, the others kept");
    // The edit undone: lane 1 again, and lane 4 superseded.
    run(a, 1, Some(1));
    assert_eq!([a, edited, b, none].map(live), [true, false, true, true], "the undone edit's lane superseded");
}

/// Decision 0302 §4: a lane recorded with no set is an older binary's (this one names every lane's set, a run with
/// none its lane's own). Over a root this binary has run, such a lane keeps no protection: no newer lane of its set can
/// ever supersede it. Over a root only older binaries ran, it is protected as they protected it.
#[test]
fn an_older_binarys_lane_keeps_no_protection_where_sets_are_named() {
    let (_path, store) = store("corpus-legacy-lanes");
    let policy = CorpusPolicy { protected_runs: 1, protected_lanes: 6, high_water: 0, low_water: 0, backstop_secs: 86_400, ..CorpusPolicy::default() };
    let live = |plan: [u8; 32]| -> bool {
        store.connection.lock().unwrap().query_row("SELECT live FROM corpus_plans WHERE plan_digest=?1", [&plan[..]], |r| r.get::<_, i64>(0)).optional().unwrap() == Some(1)
    };
    let source = store.put_source(&noise(72, 256), &[]).unwrap();
    store.put_tape(source, 1, 7, 9, &noise(572, 256)).unwrap();
    let (old_a, old_b, named, alone) = ([111u8; 32], [112u8; 32], [113u8; 32], [114u8; 32]);
    store.record_corpus_run_lane("/repo", &[source], 7, 9, old_a, &[], Some([1; 32]), 1_001).unwrap();
    store.record_corpus_run_lane("/repo", &[source], 7, 9, old_b, &[], Some([2; 32]), 1_002).unwrap();
    store.record_corpus_run_lane("/other", &[source], 7, 9, alone, &[], Some([4; 32]), 1_003).unwrap();
    store.keep_corpus_runs(&policy, 1_003).unwrap();
    assert_eq!([old_a, old_b, alone].map(live), [true; 3], "older binaries' lanes, protected as they were");
    store.record_corpus_run_policy("/repo", &[source], 7, 9, named, &[], Some([3; 32]), Some([9; 32]), 1_004).unwrap();
    store.keep_corpus_runs(&policy, 1_004).unwrap();
    assert_eq!([old_a, old_b, named, alone].map(live), [false, false, true, true], "where sets are named, the older lanes go");
}

/// Decision 0227: a run that begins stops the frames sweep (its progress handler), rolled back and still due. The
/// sweep is many small statements, each a few VM steps: the run must stop it all the same.
#[test]
fn a_run_that_begins_stops_a_sweep_of_many_small_deletes() {
    use crate::content_store::frames::{Put, Rel, Rows};
    let (_path, store) = store("corpus-sweep-stops");
    let source = store.put_source(&noise(73, 256), &[]).unwrap();
    store.record_corpus_run_naming("/r", &[source], 1, 1, [5u8; 32], &[], 1).unwrap();
    // 3,000 frames of a program no manifest names: all unreachable.
    let space = [9u8; 32];
    let rels: Vec<Rel> = (0..3000u32).map(|i| Rel { name: format!("r{i}"), ident: [1; 32] }).collect();
    let puts: Vec<Put> = rels.iter().map(|r| Put { rel: r.clone(), part: 0, digest: 1, reads: Vec::new(), rows: Rows { types: vec![0], rows: vec![1], n: 1 } }).collect();
    store.keep_frames(&space, &puts, &[], Some((&rels[..], 0, [7u8; 32]))).unwrap();
    store.keep_corpus_runs(&CorpusPolicy::default(), 2).unwrap();
    assert!(store.frames_sweep_wanted().unwrap());
    let asked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = std::sync::Arc::clone(&asked);
    let out = store.sweep_frames(move || { seen.fetch_add(1, std::sync::atomic::Ordering::Relaxed); true }).unwrap();
    assert!(out.is_none(), "a run began: the sweep stopped (asked {} times)", asked.load(std::sync::atomic::Ordering::Relaxed));
    assert!(store.frames_sweep_wanted().unwrap(), "rolled back, still due");
    let n: i64 = store.connection.lock().unwrap().query_row("SELECT count(*) FROM frames WHERE space = ?1", [&space[..]], |r| r.get(0)).unwrap();
    assert_eq!(n, 3000, "every frame still there");
}

#[test]
fn a_lanes_sources_are_its_newest_runs_whichever_build_ran_it() {
    let (_path, store) = store("corpus-lane-sources");
    let sources: Vec<SourceId> = (0..3u8).map(|i| store.put_source(&noise(80 + u64::from(i), 256), &[]).unwrap()).collect();
    assert!(store.lane_sources("/repo", &[1; 32]).unwrap().is_empty(), "a lane with no run read nothing");
    store.record_corpus_run_lane("/repo", &sources[..2], 7, 9, [10; 32], &[], Some([1; 32]), 1_000).unwrap();
    store.record_corpus_run_lane("/repo", &sources[2..], 7, 9, [11; 32], &[], Some([2; 32]), 1_001).unwrap();
    // The same lane under another build (another plan), reading one source more.
    store.record_corpus_run_lane("/repo", &sources, 7, 9, [12; 32], &[], Some([1; 32]), 1_002).unwrap();
    let mut got = store.lane_sources("/repo", &[1; 32]).unwrap();
    got.sort_by_key(|s| s.0);
    assert_eq!(got, sources, "lane 1's newest run");
    assert_eq!(store.lane_sources("/repo", &[2; 32]).unwrap(), sources[2..].to_vec(), "lane 2's own");
    assert!(store.lane_sources("/other", &[1; 32]).unwrap().is_empty(), "by root");
}

/// Decision 0227: a keep decision leaves the frames it made unreachable to the `frames` upkeep job, which is due
/// until it has swept them, and a utility job: finished by `finish_soon`, with no scheduler.
#[test]
fn a_keep_decision_leaves_its_sweep_to_the_frames_job() {
    use rerail_accel::upkeep::Job as _;
    let (_path, store) = store("corpus-frames-job");
    let policy = CorpusPolicy { protected_runs: 1, high_water: 0, low_water: 0, backstop_secs: 86_400, ..CorpusPolicy::default() };
    let mut job = crate::content_store::upkeep::Frames::new(store.database_path());
    assert_eq!(job.qos(), rerail_accel::upkeep::Qos::Utility);
    assert!(!store.frames_sweep_wanted().unwrap(), "a new store has nothing to sweep");
    assert_eq!(job.due(), rerail_accel::upkeep::Due::No);
    let source = store.put_source(&noise(7, 256), &[]).unwrap();
    store.put_tape(source, 1, 7, 9, &noise(507, 256)).unwrap();
    assert!(store.record_corpus_run("/repo", &[source], 7, 9, [7u8; 32], 1_000).unwrap());
    store.keep_corpus_runs(&policy, 1_000).unwrap();
    assert!(store.frames_sweep_wanted().unwrap(), "the decision is past the last sweep");
    assert_eq!(job.due(), rerail_accel::upkeep::Due::Urgent);
    let stop = std::sync::atomic::AtomicBool::new(false);
    assert_eq!(job.step(&rerail_accel::upkeep::Slice::for_test(std::time::Duration::from_millis(200), 64 << 20, &stop)).unwrap(), rerail_accel::upkeep::Step::Done);
    assert!(!store.frames_sweep_wanted().unwrap(), "swept");
    assert_eq!(job.due(), rerail_accel::upkeep::Due::No);
}

/// A frames job outlives its store (the process holds it): once the store is gone, the job reads no deleted file
/// and is not due, and its step makes no new store at the path. Its cached connection had read the deleted file's
/// last decision for ever, each `finish_soon` sweeping a store the step had made anew, a hundred passes over every
/// gone store of a test process.
#[test]
fn a_frames_job_over_a_gone_store_is_not_due_and_makes_no_store() {
    use rerail_accel::upkeep::Job as _;
    let (path, store) = store("corpus-frames-gone");
    let file = store.database_path().to_path_buf();
    let policy = CorpusPolicy { protected_runs: 1, high_water: 0, low_water: 0, backstop_secs: 86_400, ..CorpusPolicy::default() };
    let mut job = crate::content_store::upkeep::Frames::new(&file);
    let source = store.put_source(&noise(9, 256), &[]).unwrap();
    store.put_tape(source, 1, 7, 9, &noise(509, 256)).unwrap();
    assert!(store.record_corpus_run("/repo", &[source], 7, 9, [9u8; 32], 1_000).unwrap());
    store.keep_corpus_runs(&policy, 1_000).unwrap();
    assert_eq!(job.due(), rerail_accel::upkeep::Due::Urgent);
    drop(store);
    drop(path);
    assert!(!file.exists(), "the store's folder is gone with its binding");
    assert_eq!(job.due(), rerail_accel::upkeep::Due::No, "a gone store has nothing to sweep");
    let stop = std::sync::atomic::AtomicBool::new(false);
    assert_eq!(job.step(&rerail_accel::upkeep::Slice::for_test(std::time::Duration::from_millis(200), 64 << 20, &stop)).unwrap(), rerail_accel::upkeep::Step::Done);
    assert!(!file.exists(), "the step made no store");
}

/// Decision 0190 §8: a sweep yields to a run that begins over its store, here another process's (its shared lock on
/// `<store>.runs.lock`). The step stops with its transaction rolled back, and the sweep stays due for after the run.
#[test]
fn a_sweep_yields_to_a_run_over_its_store() {
    use rerail_accel::upkeep::Job as _;
    let (_path, store) = store("corpus-frames-yield");
    let policy = CorpusPolicy { protected_runs: 1, high_water: 0, low_water: 0, backstop_secs: 86_400, ..CorpusPolicy::default() };
    let mut job = crate::content_store::upkeep::Frames::new(store.database_path());
    let source = store.put_source(&noise(8, 256), &[]).unwrap();
    store.put_tape(source, 1, 7, 9, &noise(508, 256)).unwrap();
    assert!(store.record_corpus_run("/repo", &[source], 7, 9, [8u8; 32], 1_000).unwrap());
    store.keep_corpus_runs(&policy, 1_000).unwrap();
    assert!(store.frames_sweep_wanted().unwrap());
    let mut lock = store.database_path().as_os_str().to_owned();
    lock.push(".runs.lock");
    let run = std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(&lock).unwrap();
    run.lock_shared().unwrap();
    let stop = std::sync::atomic::AtomicBool::new(false);
    let slice = || rerail_accel::upkeep::Slice::for_test(std::time::Duration::from_millis(200), 64 << 20, &stop).over_file(store.database_path());
    assert_eq!(job.step(&slice()).unwrap(), rerail_accel::upkeep::Step::More, "the run stops the sweep");
    assert!(store.frames_sweep_wanted().unwrap(), "still due");
    run.unlock().unwrap();
    // The probe is cached for 25 ms.
    std::thread::sleep(std::time::Duration::from_millis(30));
    assert_eq!(job.step(&slice()).unwrap(), rerail_accel::upkeep::Step::Done);
    assert!(!store.frames_sweep_wanted().unwrap(), "swept once the run ended");
}

#[test]
fn a_store_mostly_free_is_left_to_the_whole_vacuum() {
    let (_path, store) = store("collect-mostly-free");
    let count = |p: &str| -> i64 { store.connection.lock().unwrap().query_row(&format!("PRAGMA {p}"), [], |r| r.get(0)).unwrap() };
    // 24 MB written and dropped: past `FREE_BYTES`, and most of the file.
    store.connection.lock().unwrap().execute_batch("CREATE TABLE ballast(b BLOB); WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 6000) INSERT INTO ballast SELECT zeroblob(4000) FROM n; DROP TABLE ballast;").unwrap();
    let free = count("freelist_count");
    assert!(free * 4 > count("page_count") * 3, "{free} free of {}", count("page_count"));
    store.collect(1_000, 64).unwrap();
    // The collector's own vacuum left them: each of its pages walks a free list this long.
    assert_eq!(count("freelist_count"), free);
}
