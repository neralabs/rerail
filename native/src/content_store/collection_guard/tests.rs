use super::*;
use crate::content_store::tests::store;
use crate::content_store::{CorpusPolicy, FactsMemoKey, FactsPlanSet, FactsResult, HashKey};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};

#[test]
fn readers_publish_memos_and_roots_before_collection_resumes() {
    let (path, writer) = store("collection-readers");
    let collector = ContentStore::open(&path).unwrap();
    let source = writer.put_source(b"in-flight source", &[]).unwrap();
    let orphan = writer.put_source(b"unreferenced source", &[]).unwrap();
    writer.put_tape(source, 1, 7, 9, b"tape").unwrap();
    // Advance the generation so the next collection would delete these unrooted objects.
    collector.collect(100, 100).unwrap();
    let first = writer.hold_collection().unwrap();
    let second = collector.hold_collection().unwrap();
    assert!(collector.try_collect().unwrap().is_none());
    let policy = CorpusPolicy::default();
    let plan = [1; 32];
    // Another corpus finishing schedules a collection while this one still owns unrooted ids.
    collector
        .record_corpus_run("/other", &[], 7, 9, [2; 32], 100)
        .unwrap();
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                for _ in 0..3 {
                    assert_eq!(collector.collect(100, 100).unwrap().sources, 0);
                    assert!(
                        !collector
                            .collect_corpus_pass(&policy, 100)
                            .unwrap()
                            .did_work()
                    );
                    assert_eq!(collector.keep_corpus_runs(&policy, 100).unwrap().evicted, 0);
                }
            })
            .join()
            .unwrap();
    });
    assert!(
        collector.corpus_collection_due().unwrap(),
        "a deferred pass must remain due"
    );
    let key = FactsMemoKey {
        source,
        fact_schema: 1,
        path: "a.py",
    };
    writer
        .put_facts_memo(
            key,
            &FactsPlanSet::new(vec![plan]),
            &[FactsResult {
                plan,
                path_dependent: false,
                entry: b"result".to_vec(),
            }],
            &[],
        )
        .unwrap();
    writer
        .record_corpus_run("/repo", &[source], 7, 9, plan, 100)
        .unwrap();
    drop(first);
    assert!(
        collector.try_collect().unwrap().is_none(),
        "every reader must finish"
    );
    drop(second);
    for _ in 0..3 {
        collector.collect_corpus_pass(&policy, 100).unwrap();
    }
    assert!(!collector.corpus_collection_due().unwrap());
    assert_eq!(
        writer.facts_memo(key).unwrap().results[&plan].entry,
        b"result"
    );
    assert!(writer.tape_by_source(source, 1, 7, 9).unwrap().is_some());
    assert_eq!(
        writer
            .source_by_hash(HashKey::sha256(b"unreferenced source"))
            .unwrap(),
        None
    );
    assert_ne!(source, orphan);
}

const CHILD_TEST: &str = "content_store::collection_guard::tests::lease_process";
const CHILD_DATABASE: &str = "RERAIL_TEST_COLLECTION_LEASE_DATABASE";

#[test]
fn releasing_a_lease_unlocks_inherited_descriptors() {
    let (path, reader) = store("collection-inherited");
    let collector = ContentStore::open(&path).unwrap();
    for exclusive in [false, true] {
        let lease = if exclusive {
            reader.try_collect().unwrap().unwrap()
        } else {
            reader.hold_collection().unwrap()
        };
        // A duplicate shares the open file description, just as a child inherits it at fork.
        let inherited = lease.duplicate();
        assert!(collector.try_collect().unwrap().is_none());
        drop(lease);
        assert!(collector.try_collect().unwrap().is_some());
        drop(inherited);
    }
}

// Also runs as an ordinary test, where it has no work. Only the child gets the database argument.
#[test]
fn lease_process() {
    let Some(path) = std::env::var_os(CHILD_DATABASE) else {
        return;
    };
    let store = ContentStore::open(std::path::Path::new(&path)).unwrap();
    let _reading = store.hold_collection().unwrap();
    println!("LEASE_READY");
    std::io::stdout().flush().unwrap();
    let mut byte = [0];
    std::io::stdin().read_exact(&mut byte).unwrap();
}

struct KillOnDrop(Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn process_exit_releases_the_lease_and_aliases_share_it() {
    let (path, collector) = store("collection-process");
    collector.put_source(b"unrooted", &[]).unwrap();
    collector.collect(100, 100).unwrap();
    #[cfg(unix)]
    let child_path = {
        let alias = path.with_extension("alias.sqlite");
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        alias
    };
    #[cfg(not(unix))]
    let child_path = path.to_path_buf();
    let mut child = KillOnDrop(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", CHILD_TEST, "--nocapture"])
            .env(CHILD_DATABASE, &child_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut output = BufReader::new(child.0.stdout.take().unwrap());
    loop {
        let mut line = String::new();
        assert_ne!(
            output.read_line(&mut line).unwrap(),
            0,
            "child exited before taking the lease"
        );
        if line.contains("LEASE_READY") {
            break;
        }
    }
    assert!(
        collector.try_collect().unwrap().is_none(),
        "a reader in another process protects this store"
    );
    assert_eq!(collector.collect(100, 100).unwrap().sources, 0);
    // Kill rather than exiting normally: a crashed analysis must not leave persistent pins.
    drop(child);
    assert_eq!(collector.collect(100, 100).unwrap().sources, 1);
    #[cfg(unix)]
    std::fs::remove_file(child_path).unwrap();
}
