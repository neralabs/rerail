use super::*;
use crate::content_store::tests::store;

fn manifest(id: i64, root: i64, last_time: i64, sources: &[i64]) -> Manifest {
    Manifest { id, root, last_time, grammar_version: 1, tape_format: 1, plan: vec![id as u8; 32], sources: sources.to_vec() }
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
fn the_last_runs_are_protected_and_history_is_kept_to_the_low_water_mark() {
    // Root 1 churns every file each run: sources 1-2, 3-4, 5-6, 7-8, 9-10 (newest first below).
    let ms: Vec<Manifest> = (0..5).map(|k| manifest(10 - k, 1, 100, &[2 * (5 - k) - 1, 2 * (5 - k)])).collect();
    let s = sizes(10, 10);
    // Each manifest holds 40 bytes alone. Two protected; history of three is 120 bytes.
    let policy = CorpusPolicy { protected_runs: 2, high_water: 1000, low_water: 50, backstop_secs: 1000 };
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
    let policy = CorpusPolicy { protected_runs: 1, high_water: 1000, low_water: 500, backstop_secs: 50 };
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
    let policy = CorpusPolicy { protected_runs: 2, high_water: 64 << 10, low_water: 48 << 10, backstop_secs: 86_400 };
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
