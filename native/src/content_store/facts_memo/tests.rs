use super::super::schema::SCHEMA_VERSION;
use super::super::*;
use rusqlite::Connection;
use std::time::{SystemTime, UNIX_EPOCH};

fn store(label: &str) -> (std::path::PathBuf, ContentStore) {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!(
        "rerail-facts-memo-{label}-{}-{nonce}.sqlite",
        std::process::id()
    ));
    let store = ContentStore::open(&path).unwrap();
    (path, store)
}

fn plan(n: u8) -> [u8; 32] {
    [n; 32]
}

fn content(bytes: &[u8]) -> [u8; 32] {
    // Any 32-byte key works for the store; the engine passes blake3.
    HashKey::sha256(bytes).sha256_bytes().unwrap()
}

fn count(store: &ContentStore, table: &str) -> i64 {
    let connection = store.connection.lock().unwrap();
    connection
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row.get(0))
        .unwrap()
}

fn result(p: u8, path_dependent: bool, entry: &[u8]) -> FactsResult {
    FactsResult { plan: plan(p), path_dependent, entry: entry.to_vec() }
}

fn completion(ordinal: u32, path_dependent: bool, counts: &[u8]) -> FactsCompletion {
    FactsCompletion { ordinal, path_dependent, counts: counts.to_vec() }
}

#[test]
fn content_pure_rows_serve_every_path_and_path_rows_only_theirs() {
    let (_path, store) = store("paths");
    let bytes = b"def f():\n    pass\n";
    let source = store.put_facts_source(bytes, content(bytes), &[]).unwrap();
    assert_eq!(store.facts_source(content(bytes)).unwrap(), Some(source));
    assert_eq!(store.facts_source(content(b"other")).unwrap(), None);
    let set = FactsPlanSet::new(vec![plan(1), plan(2), plan(3), plan(4)]);
    let at_a = FactsMemoKey { source, fact_schema: 7, path: "a/x.py" };
    store
        .put_facts_memo(
            at_a,
            &set,
            &[result(1, false, b"pure finding"), result(2, true, b"path finding")],
            &[completion(2, false, b"c3"), completion(3, true, b"c4")],
        )
        .unwrap();

    let read_a = store.facts_memo(at_a).unwrap();
    assert_eq!(read_a.results.len(), 2);
    assert_eq!(read_a.results[&plan(1)].entry, b"pure finding");
    assert!(!read_a.results[&plan(1)].path_dependent);
    assert!(read_a.results[&plan(2)].path_dependent);
    assert_eq!(read_a.completed[&plan(3)], (false, b"c3".to_vec()));
    assert_eq!(read_a.completed[&plan(4)], (true, b"c4".to_vec()));

    // Identical content at another path: the content-pure part only.
    let read_b = store.facts_memo(FactsMemoKey { path: "b/y.py", ..at_a }).unwrap();
    assert_eq!(read_b.results.keys().collect::<Vec<_>>(), vec![&plan(1)]);
    assert_eq!(read_b.completed.keys().collect::<Vec<_>>(), vec![&plan(3)]);

    // Another fact schema version is another key.
    assert!(store.facts_memo(FactsMemoKey { fact_schema: 8, ..at_a }).unwrap().is_empty());
}

#[test]
fn completion_words_merge_and_serve_other_plan_sets() {
    let (_path, store) = store("merge");
    let bytes = b"x = 1\n";
    let source = store.put_facts_source(bytes, content(bytes), &[]).unwrap();
    let key = FactsMemoKey { source, fact_schema: 1, path: "x.py" };
    // 70 plans: completions span two words.
    let old = FactsPlanSet::new((0..70).map(plan).collect());
    store.put_facts_memo(key, &old, &[], &[completion(0, false, b"a")]).unwrap();
    store.put_facts_memo(key, &old, &[], &[completion(69, false, b"b")]).unwrap();
    let memo = store.facts_memo(key).unwrap();
    assert_eq!(memo.completed.len(), 2);
    assert_eq!(memo.completed[&plan(0)].1, b"a");
    assert_eq!(memo.completed[&plan(69)].1, b"b");
    assert_eq!(count(&store, "facts_completions"), 1);

    // A set that changed one plan (plan 0 → 200) reuses the others by digest.
    let mut plans: Vec<[u8; 32]> = (0..70).map(plan).collect();
    plans[0] = plan(200);
    let new = FactsPlanSet::new(plans);
    assert_ne!(new.digest, old.digest);
    store.put_facts_memo(key, &new, &[], &[completion(0, false, b"n")]).unwrap();
    let memo = store.facts_memo(key).unwrap();
    assert_eq!(memo.completed[&plan(200)].1, b"n");
    assert_eq!(memo.completed[&plan(69)].1, b"b");
    assert_eq!(memo.completed[&plan(0)].1, b"a");

    // A result row wins over a completion bit of another set for the same plan.
    store.put_facts_memo(key, &new, &[result(69, false, b"found")], &[]).unwrap();
    let memo = store.facts_memo(key).unwrap();
    assert_eq!(memo.results[&plan(69)].entry, b"found");
    assert!(!memo.completed.contains_key(&plan(69)));
}

#[test]
fn completions_outside_their_set_and_empty_paths_are_refused() {
    let (_path, store) = store("refused");
    let source = store.put_facts_source(b"y", content(b"y"), &[]).unwrap();
    let set = FactsPlanSet::new(vec![plan(1)]);
    let key = FactsMemoKey { source, fact_schema: 1, path: "y.py" };
    assert!(matches!(
        store.put_facts_memo(key, &set, &[], &[completion(1, false, b"")]),
        Err(StoreError::Invalid(_))
    ));
    assert!(store.put_facts_memo(FactsMemoKey { path: "", ..key }, &set, &[], &[]).is_err());
}

/// Memo rows die with their source, whichever binary collects it; pinned sources survive; a plan
/// set no completion names is collected.
#[test]
fn memo_rows_cascade_from_collected_sources_and_pins_keep_them() {
    let (_path, store) = store("cascade");
    let set = FactsPlanSet::new(vec![plan(1), plan(2)]);
    let kept = store.put_facts_source(b"kept", content(b"kept"), &[]).unwrap();
    let dropped = store.put_facts_source(b"dropped", content(b"dropped"), &[]).unwrap();
    for source in [kept, dropped] {
        store
            .put_facts_memo(
                FactsMemoKey { source, fact_schema: 1, path: "p.py" },
                &set,
                &[result(1, true, b"r")],
                &[completion(1, false, b"c")],
            )
            .unwrap();
    }
    store.pin_facts_sources(&[kept], "facts-daemon", i64::MAX).unwrap();
    for _ in 0..3 {
        store.collect(1, 100).unwrap();
    }
    assert_eq!(store.facts_source(content(b"dropped")).unwrap(), None);
    assert_eq!(store.facts_source(content(b"kept")).unwrap(), Some(kept));
    assert_eq!(count(&store, "facts_results"), 1);
    assert_eq!(count(&store, "facts_completions"), 1);
    assert_eq!(count(&store, "facts_plan_sets"), 2);

    // Unpinned, the last source goes with its rows and then its plan set.
    store.unpin_facts_sources(&[kept], "facts-daemon").unwrap();
    for _ in 0..3 {
        store.collect(1, 100).unwrap();
    }
    assert_eq!(count(&store, "facts_results"), 0);
    assert_eq!(count(&store, "facts_completions"), 0);
    assert_eq!(count(&store, "facts_plan_sets"), 0);
}

/// A store this build did not create (an older binary's, at the same floor, without the facts
/// tables) gains them on open without a floor raise, and keeps what it held.
#[test]
fn opening_a_store_without_the_facts_tables_adds_them_at_the_same_floor() {
    let (path, store) = store("migration");
    let old = store.put_source(b"older source", &[]).unwrap();
    drop(store);
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "DROP INDEX facts_completions_by_set;
             DROP TABLE facts_completions;
             DROP TABLE facts_results;
             DROP TABLE facts_plan_sets;",
        )
        .unwrap();
    drop(connection);

    let store = ContentStore::open(&path).unwrap();
    {
        let connection = store.connection.lock().unwrap();
        let floor: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(floor, SCHEMA_VERSION);
        assert_eq!(SCHEMA_VERSION, 1, "the facts memo is additive: no floor raise");
        assert_eq!(STORE_FILE_NAME, "content-v1.sqlite");
    }
    assert_eq!(store.get_source(old).unwrap(), b"older source");
    let source = store.put_facts_source(b"older source", content(b"older source"), &[]).unwrap();
    assert_eq!(source, old, "the facts alias attaches to the stored source");
    let set = FactsPlanSet::new(vec![plan(9)]);
    let key = FactsMemoKey { source, fact_schema: 1, path: "o.py" };
    store.put_facts_memo(key, &set, &[result(9, false, b"r")], &[]).unwrap();
    assert_eq!(store.facts_memo(key).unwrap().results[&plan(9)].entry, b"r");
}

/// The other direction: a binary opening a store a newer build extended with tables it does not
/// know reads it as is (no reapply, no floor change), which is what an older binary does with the
/// facts tables.
#[test]
fn unknown_additive_tables_leave_the_store_readable() {
    let (path, store) = store("forward");
    let source = store.put_source(b"s", &[]).unwrap();
    drop(store);
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch("CREATE TABLE future_facts (source_id INTEGER REFERENCES source_objects(source_id) ON DELETE CASCADE) STRICT;")
        .unwrap();
    connection.execute("INSERT INTO future_facts VALUES (?1)", [source.0]).unwrap();
    drop(connection);
    let store = ContentStore::open(&path).unwrap();
    assert_eq!(store.get_source(source).unwrap(), b"s");
    for _ in 0..3 {
        store.collect(1, 100).unwrap();
    }
    assert_eq!(count(&store, "future_facts"), 0, "the cascade follows collection");
}

#[test]
fn a_run_digest_names_the_output_not_the_content() {
    let (_path, store) = store("run-digest");
    let set = FactsPlanSet::new(vec![plan(1), plan(2)]);
    let results = [result(1, false, b"finding")];
    let completions = [completion(1, true, b"c2")];
    let mut digests = Vec::new();
    for bytes in [&b"a = 1\n"[..], b"a = 1  \n"] {
        let source = store.put_facts_source(bytes, content(bytes), &[]).unwrap();
        let key = FactsMemoKey { source, fact_schema: 1, path: "a.py" };
        let put = store.put_facts_memo(key, &set, &results, &completions).unwrap();
        assert_eq!(put, store.facts_memo(key).unwrap().run_digest(&set));
        digests.push(put.unwrap());
    }
    assert_eq!(digests[0], digests[1]);
    // A run that leaves a clause unknown has no digest; a different report has another.
    let bytes = b"a = 2\n";
    let source = store.put_facts_source(bytes, content(bytes), &[]).unwrap();
    let key = FactsMemoKey { source, fact_schema: 1, path: "a.py" };
    assert_eq!(store.put_facts_memo(key, &set, &results, &[]).unwrap(), None);
    let other = store.put_facts_memo(key, &set, &[result(1, false, b"other")], &completions).unwrap();
    assert_ne!(other, Some(digests[0]));
}
