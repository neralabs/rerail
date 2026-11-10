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
        // The facts memo is additive: it raised no floor (3 is the atom segments', decision 0049).
        assert_eq!(SCHEMA_VERSION, 3);
        assert_eq!(STORE_FILE_NAME, "content-v3.sqlite");
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

#[test]
fn a_memo_row_that_does_not_decode_is_a_miss_and_is_replaced() {
    // O112: a row whose frame decodes to another length than its metadata fails only its own plans.
    let (_path, store) = store("undecodable");
    let bytes = b"x = 1\n";
    let source = store.put_facts_source(bytes, content(bytes), &[]).unwrap();
    let set = FactsPlanSet::new(vec![plan(1), plan(2), plan(3)]);
    let key = FactsMemoKey { source, fact_schema: 7, path: "a.py" };
    store
        .put_facts_memo(key, &set, &[result(1, false, b"one"), result(2, false, b"two")], &[completion(2, false, b"c3")])
        .unwrap();
    {
        let connection = store.connection.lock().unwrap();
        connection
            .execute("UPDATE facts_results SET raw_length=raw_length+1 WHERE plan_digest=?1", [&plan(1)[..]])
            .unwrap();
        connection.execute("UPDATE facts_completions SET raw_length=raw_length+1", []).unwrap();
    }
    let read = store.facts_memo(key).unwrap();
    assert_eq!(read.results.keys().collect::<Vec<_>>(), vec![&plan(2)]);
    assert!(read.completed.is_empty());
    assert_eq!(read.run_digest(&set), None);

    // Evaluated again, the clauses' rows replace the bad ones.
    store
        .put_facts_memo(key, &set, &[result(1, false, b"one")], &[completion(2, false, b"c3")])
        .unwrap();
    let read = store.facts_memo(key).unwrap();
    assert_eq!(read.results[&plan(1)].entry, b"one");
    assert_eq!(read.completed[&plan(3)], (false, b"c3".to_vec()));
    assert!(read.run_digest(&set).is_some());
}

#[test]
fn one_plans_result_is_the_memos_row_for_it() {
    let (_path, store) = store("one-plan");
    let bytes = b"x = 1\n";
    let source = store.put_facts_source(bytes, content(bytes), &[]).unwrap();
    let set = FactsPlanSet::new(vec![plan(1), plan(2), plan(3)]);
    let at_a = FactsMemoKey { source, fact_schema: 7, path: "a/x.py" };
    store.put_facts_memo(at_a, &set, &[result(1, false, b"pure"), result(2, true, b"at a")], &[]).unwrap();
    for (key, p) in [(at_a, 1), (at_a, 2), (at_a, 3), (FactsMemoKey { path: "b/y.py", ..at_a }, 1), (FactsMemoKey { path: "b/y.py", ..at_a }, 2)] {
        let whole = store.facts_memo(key).unwrap();
        let one = store.facts_result(key, plan(p)).unwrap();
        assert_eq!(one.as_ref().map(|r| (&r.entry, r.path_dependent)), whole.results.get(&plan(p)).map(|r| (&r.entry, r.path_dependent)), "{} plan {p}", key.path);
    }
}

/// Many files' results read and written as one wave answer as `facts_memo` and `put_facts_memo` do one file at a
/// time: in the keys' order, a path's own result over a content-pure one, nothing for a file without one; a source
/// collected after it was resolved is not written; sources found by hash in the hashes' order.
#[test]
fn results_for_many_files_answer_as_one_file_at_a_time() {
    let (_path, store) = store("batch");
    let (a, b) = (b"a = 1\n".as_slice(), b"b = 2\n".as_slice());
    let sa = store.put_facts_source(a, content(a), &[]).unwrap();
    let sb = store.put_facts_source(b, content(b), &[]).unwrap();
    let set = FactsPlanSet::new(vec![plan(1)]);
    let gone = SourceId(sb.0 + 100);
    let written = store
        .put_facts_results(7, &set, &[(sa, "a.py", result(1, true, b"a at a")), (sb, "b.py", result(1, false, b"b pure")), (gone, "c.py", result(1, true, b"no source"))])
        .unwrap();
    assert_eq!(written, 2);
    store.put_facts_results(7, &set, &[(sb, "b.py", result(1, true, b"b at b"))]).unwrap();
    let found = store.facts_results_for(7, plan(1), &[(sb, "b.py"), (sa, "elsewhere.py"), (sa, "a.py"), (sb, "other.py"), (gone, "c.py")]).unwrap();
    assert_eq!(found, vec![Some(b"b at b".to_vec()), None, Some(b"a at a".to_vec()), Some(b"b pure".to_vec()), None]);
    assert_eq!(store.facts_results_for(8, plan(1), &[(sa, "a.py")]).unwrap(), vec![None]);
    let hashes = [HashKey::sha256(b), HashKey::sha256(b"absent"), HashKey::sha256(a)];
    assert_eq!(store.held_sources_by_hash(&hashes).unwrap(), vec![store.source_by_hash(hashes[0]).unwrap(), None, store.source_by_hash(hashes[2]).unwrap()]);
}
