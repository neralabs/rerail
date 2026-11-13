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

/// O95: a handle holding its sources pins each one it stores or finds, in the same transaction; a collector cannot
/// take them, or the tapes it stores of them, until the holder releases them.
#[test]
fn a_held_source_survives_collection_until_released() {
    let (path, store) = store("corpus-held-source");
    let collector = ContentStore::open(&path).unwrap();
    let passes = || {
        for _ in 0..3 {
            collector.collect(1_000, MAX_COLLECTION_BATCH).unwrap();
        }
    };
    let found = noise(7, 256);
    let old = store.put_source(&found, &[]).unwrap();
    store.hold_sources("corpus-run:held", 2_000).unwrap();
    let stored = noise(8, 256);
    let (new, tape) = store.put_source_and_tape(&stored, crate::content_store::HashKey::sha256(&stored), 1, 7, 9, &noise(9, 256)).unwrap();
    assert_eq!(store.held_source_by_hash(crate::content_store::HashKey::sha256(&found)).unwrap(), Some(old));
    passes();
    assert_eq!(store.tape_by_source(new, 1, 7, 9).unwrap(), Some(tape), "the stored source and its tape are held");
    assert_eq!(store.source_by_hash(crate::content_store::HashKey::sha256(&found)).unwrap(), Some(old), "the found source is held");
    store.release_pins("corpus-run:held").unwrap();
    passes();
    assert_eq!(store.source_by_hash(crate::content_store::HashKey::sha256(&found)).unwrap(), None, "released, it is collected");
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
