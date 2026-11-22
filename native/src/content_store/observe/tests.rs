//! The observer's tests (`observe.rs`). They read the content store, which is above the observer's crate: the
//! store hosts them (rerail-store, `observe_host`), and registers the kept facts they read.

use super::*;

/// How long a test's journal may take to answer (`journal_observer`).
const JOURNAL_TEST_BUDGET: std::time::Duration = std::time::Duration::from_secs(20);

/// The settings kept for the observer at `store` (`facts`).
fn kept_meta(store: &Path) -> Meta {
    facts::Facts::open(store).map(|f| f.meta()).unwrap_or_default()
}

/// The keys of the settings kept for the observer at `store` that start with `prefix`, sorted.
fn meta_keys(store: &Path, prefix: &str) -> Vec<String> {
    let mut keys: Vec<String> = kept_meta(store).into_keys().filter(|k| k.starts_with(prefix)).collect();
    keys.sort();
    keys
}

/// The setting `key` kept for the observer at `store`, as a cursor.
fn kept_cursor(store: &Path, key: &str) -> Option<String> {
    kept_meta(store).get(key).and_then(text_cursor)
}

/// Keeps `value` as the setting `key` of the observer at `store`, as another process would.
fn put_meta(store: &Path, key: String, value: rusqlite::types::Value) {
    let f = facts::Facts::open(store).expect("the observer's facts");
    f.apply(facts::Change { meta_put: vec![(key, value)], ..Default::default() }, "kept facts");
    f.written();
}

/// The paths of the answers kept for the observer at `store`, sorted.
fn kept_answers(store: &Path) -> Vec<String> {
    facts::Facts::open(store).map(|f| facts::all_paths(&f)).unwrap_or_default()
}

/// The paths of the canonical forms kept for the observer at `store`, sorted.
fn kept_forms(store: &Path) -> Vec<String> {
    facts::Facts::open(store).map(|f| facts::all_canonical(&f)).unwrap_or_default()
}

/// Keeps `canonical` as `path`'s canonical form, recorded at `recorded_ns`, as another process would.
fn put_form(store: &Path, path: &Path, canonical: &Path, recorded_ns: i64) {
    let f = facts::Facts::open(store).expect("the observer's facts");
    let form = (stored(path), canonical.to_string_lossy().into_owned(), recorded_ns);
    f.apply(facts::Change { canon: facts::Edit { put: vec![form], ..Default::default() }, ..Default::default() }, "kept facts");
    f.written();
}

/// Keeps an answer for `path` (a file, no stat, no digest) recorded at `recorded_ns`, as another process would.
fn put_answer(store: &Path, path: &Path, recorded_ns: i64) {
    let f = facts::Facts::open(store).expect("the observer's facts");
    let stat = Stat { dev: 0, ino: 0, size: 0, mtime_ns: 0, ctime_ns: 0 };
    let answer = (stored(path), Kind::File.code(), stat, Vec::new(), None, recorded_ns);
    f.apply(facts::Change { answers: facts::Edit { put: vec![answer], ..Default::default() }, ..Default::default() }, "kept facts");
    f.written();
}

/// An observer that asks the journal for every root (decision 0105), as these tests were written for: no root
/// is checked by its listings (decision 0400), whose tests open `Observer::open`.
fn journal_observer(store: &Path) -> Observer {
    let mut o = Observer::open(Some(store));
    o.list_max = 0;
    // The journal's answer, however late: on a loaded host `fseventsd` was seen to deliver a stream's cookie, or
    // to flush the run's marker into its history, more than 5 s late (Q223), and the root was then `Lost`, checked
    // by its `stat`s. What the tests check is the journal's answer.
    o.journal_settle = JOURNAL_TEST_BUDGET;
    // Trust at once: these runs follow each other within the settle window, as no test can wait it out; and the
    // journal's answer whatever it costs: the cookie waited for up to 30 s, past the replay's settle window (a busy
    // host's `fseventsd` takes seconds, Q221; 2 s failed four of these on a loaded host, 7ab606e746). The wait ends
    // when the cookie comes.
    o.settle_ns = 0;
    o.wait_least_ns = 30_000_000_000;
    o
}

/// Whether the journal answered every root of the test's runs so far (`runs`, each run's counts; `Counts::late`).
/// On a loaded host it may not (Q223): `fseventsd`, one thread at a full core, delivers the machine's file events
/// in order and logs them as late, and a cookie or a marker waited behind them 20 s and more, past
/// `JOURNAL_TEST_BUDGET`. The root was then checked by `stat` or its listings: the run's answers still hold and
/// are checked; what the journal would have said is not, and the test says why. A late run reaches the next: its
/// stream answers from its sync on, and the next run's question from the stored cursor before it is a replay of
/// the history, which misses a change merged into a record older than the cursor (decision 0400), as a new
/// process's would; a run covers that by its settle window, which these runs turn off (`journal_observer`).
#[cfg(target_os = "macos")]
fn journal_answered(runs: &[&Counts], what: &str) -> bool {
    let late: u64 = runs.iter().map(|c| c.late).sum();
    if late == 0 {
        return true;
    }
    eprintln!("{what}: the journal was behind for {late} roots of this test's runs (fseventsd behind the host's file events, Q223): checked by stat or listings, so the journal's own counts are not checked here ({})", runs.last().map(|c| c.to_string()).unwrap_or_default());
    false
}

/// `under` answers as `Path::starts_with` for every pair of a set of paths, odd spellings among them.
#[test]
fn under_answers_as_path_starts_with() {
    let paths = ["/", "/a", "/a/", "/a/b", "/ab", "/a/b/c", "/a//b", "/a/./b", "/a/.b", "a", "a/b", "ab", "./a", "", "/b/a"];
    for p in paths {
        for r in paths {
            assert_eq!(under(Path::new(p), Path::new(r)), Path::new(p).starts_with(r), "{p:?} under {r:?}");
        }
    }
}

/// Decision 0105: a run that watches a root does no `stat` of what the journal says is unchanged. A first run
/// checks every path; the next, with nothing changed, none, and its answers are the same; after an edit, the
/// edited file is read again and the others are trusted. Each run is a new process's observer.
#[cfg(target_os = "macos")]
#[test]
fn a_watched_root_is_not_statted_where_the_journal_says_nothing_changed() {
    let base = std::env::temp_dir().join(format!("rerail-watch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let root = base.join("root");
    std::fs::create_dir_all(root.join("pkg")).unwrap();
    let files: Vec<PathBuf> = (0..5).map(|i| root.join(format!("pkg/m{i}.py"))).collect();
    for f in &files {
        std::fs::write(f, "x = 1\n").unwrap();
        aged(f);
    }
    let store = base.join("store.files");
    let run = |files: &[PathBuf], settle: i64| {
        let mut o = journal_observer(&store);
        o.settle_ns = settle;
        o.begin();
        o.watch(std::slice::from_ref(&root));
        let listing = o.listing(&root.join("pkg")).unwrap();
        let digests: Vec<[u8; 32]> = files.iter().map(|f| o.digest(f).unwrap()).collect();
        o.flush();
        (o.counts, listing, digests)
    };
    let (first, listing, digests) = run(&files, 0);
    assert!(first.stats >= 6 && first.trusted == 0, "the first run checks every path: {first}");
    // The setup's writes may be named once more (their events may come after the first run's cursor).
    std::thread::sleep(std::time::Duration::from_millis(300));
    let (middle, _, _) = run(&files, 0);
    let (second, again, same) = run(&files, 0);
    assert_eq!((again, same), (listing, digests.clone()));
    if journal_answered(&[&first, &middle, &second], "nothing changed") {
        assert_eq!((second.stats, second.trusted), (0, 6), "nothing changed: no stat ({second})");
    }
    std::fs::write(&files[2], "x = 2\n").unwrap();
    aged(&files[2]);
    let (third, _, mut after) = run(&files, 0);
    if journal_answered(&[&first, &middle, &second, &third], "an edit") {
        assert!(third.forgotten >= 1 && third.hashed == 1, "the edited file is read again: {third}");
    } else {
        // As a run asks, its settle window on (`journal_answered`): the edit is read whatever the journal said.
        after = run(&files, JOURNAL_SETTLE_NS).2;
    }
    assert_ne!(after[2], digests[2], "the edit is read");
    assert_eq!([after[0], after[1], after[3], after[4]], [digests[0], digests[1], digests[3], digests[4]]);
    let _ = std::fs::remove_dir_all(&base);
}

/// Q219: a root watched through `/./` (`…/q179/./work/r3/corpus-dl`, its files read by that spelling) is told an
/// edit under it, as a root spelled plainly is: the edited file is read again, the others trusted.
#[cfg(target_os = "macos")]
#[test]
fn a_root_watched_through_a_dot_reads_an_edit_under_it() {
    let base = std::env::temp_dir().join(format!("rerail-watch-dot-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("work/root/pkg")).unwrap();
    let root = base.join(".").join("work/root");
    assert!(root.to_str().unwrap().contains("/./"), "{root:?}");
    let files: Vec<PathBuf> = (0..3).map(|i| root.join(format!("pkg/m{i}.py"))).collect();
    for f in &files {
        std::fs::write(f, "x = 1\n").unwrap();
        aged(f);
    }
    let store = base.join("store.files");
    let run = || {
        let mut o = journal_observer(&store);
        o.begin();
        o.watch(std::slice::from_ref(&root));
        let digests: Vec<[u8; 32]> = files.iter().map(|f| o.digest(f).unwrap()).collect();
        o.flush();
        (o.counts, digests)
    };
    let (_, digests) = run();
    std::thread::sleep(std::time::Duration::from_millis(300));
    run();
    let (second, same) = run();
    assert_eq!(same, digests);
    std::fs::write(&files[1], "x = 2\n").unwrap();
    aged(&files[1]);
    let (third, after) = run();
    assert_ne!(after[1], digests[1], "the edit under the dotted root is read ({second}, then {third})");
    assert_eq!([after[0], after[2]], [digests[0], digests[2]]);
    let _ = std::fs::remove_dir_all(&base);
}

/// Q189: FSEvents' history can merge a change made just after a run read a path into the path's earlier event,
/// replayed under an id before the run's cursor: the next run is told nothing. An answer recorded within the
/// settle window is then checked by `stat`, and the edit read; with no window (as before) it was missed. The merge
/// is made here by moving the root's stored cursor past the edit's event, which is what the replay amounts to.
#[cfg(target_os = "macos")]
#[test]
#[ignore = "Q293: since 0400 a replay answers by arrival, so moving the cursor past the edit no longer hides it (the stream names it); the first leg's miss cannot happen"]
    fn an_answer_recorded_within_the_settle_window_is_checked_though_the_journal_names_nothing() {
    let base = std::env::temp_dir().join(format!("rerail-settle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let root = base.join("root");
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("m.py");
    std::fs::write(&file, "x = 1\n").unwrap();
    aged(&file);
    let store = base.join("store.files");
    let run = |settle: i64| {
        let mut o = Observer::open(Some(&store));
        // The journal's path (Q189): a root this small is otherwise checked by its listings (decision 0400).
        o.list_max = 0;
        o.journal_settle = JOURNAL_TEST_BUDGET;
        o.settle_ns = settle;
        o.wait_least_ns = 30_000_000_000;
        o.begin();
        o.watch(std::slice::from_ref(&root));
        let digest = o.digest(&file).unwrap();
        o.flush();
        o.written();
        (o.counts, digest)
    };
    // Recorded, then asked again so the root holds a cursor and the answer is kept.
    let (recorded, first) = run(0);
    std::thread::sleep(std::time::Duration::from_millis(300));
    let (asked, _) = run(0);
    // The edit, then the cursor moved past its event: the journal names nothing since.
    let edit_and_hide = |text: &str| {
        let before = super::super::journal::current_id();
        std::fs::write(&file, text).unwrap();
        aged(&file);
        let volume = super::super::journal::volume(&root).expect("a volume");
        // An id past the edit's events, which a loaded host may give later than the edit (Q223): a live stream's
        // sync, so a question from it is told what came after that sync alone, as a merged replay tells it.
        let Some(id) = super::super::journal::id_past(&root, before, &cookie_dir(&store)) else {
            eprintln!("the journal gave no id past the edit (later than its wait: fseventsd behind, Q223): the edit cannot be hidden");
            return false;
        };
        put_meta(&store, cursor_key(&root, volume), rusqlite::types::Value::Text(id.to_string()));
        // As the next process: its replay starts at the stored cursor. This process's stream saw the edit come,
        // and answers by when events came (decision 0400), so it would name the edit whatever the cursor.
        super::super::journal::forget_streams();
        true
    };
    if !edit_and_hide("x = 22\n") {
        let _ = std::fs::remove_dir_all(&base);
        return;
    }
    let (missed, stale) = run(0);
    // The miss shows only where the journal names nothing after the hidden cursor: its replay may name the edit
    // anyway (its event, merged into the path's earlier one, told after the sync's cookie: Q189), and the run
    // then reads it, which is the safe side. The window's check, below, holds either way.
    if !journal_answered(&[&recorded, &asked, &missed], "the journal's silence") {
    } else if missed.forgotten > 0 {
        eprintln!("the journal named the edit after the hidden cursor (its event told after the sync's cookie): no miss to show here ({missed})");
    } else {
        assert_eq!((stale, missed.hashed), (first, 0), "with no window the journal's silence is trusted: the edit is missed ({missed})");
    }
    if !edit_and_hide("x = 333\n") {
        let _ = std::fs::remove_dir_all(&base);
        return;
    }
    let (checked, fresh) = run(JOURNAL_SETTLE_NS);
    assert!(checked.stats >= 1 && checked.hashed == 1, "within the window the answer is checked and the edit read: {checked}");
    assert_ne!(fresh, first);
    let _ = std::fs::remove_dir_all(&base);
}

/// Decision 0284: rerail's own files are outside the observer. Under a watched root: the content store and every
/// file named after it (its WAL, the observer's store `<store>.files` and its cookies, the lock and upkeep files),
/// and a directory a run wrote its outputs into (recorded by the run that did, `own_output`, for later processes).
/// A listing leaves them out, no answer is kept for them, and the journal's news of them forgets nothing and moves
/// no stored cursor: a run after the writes of rerail's own is as warm as one after none.
#[cfg(target_os = "macos")]
#[test]
fn rerails_own_files_are_outside_the_observer() {
    let base = std::env::temp_dir().join(format!("rerail-own-files-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("root/pkg")).unwrap();
    let root = dunce::canonicalize(base.join("root")).unwrap();
    let source = root.join("pkg/m.py");
    std::fs::write(&source, "x = 1\n").unwrap();
    aged(&source);
    let content = root.join("store.sqlite");
    let out = root.join("out");
    // What a run writes of its own: the store and the files beside it (a real store's: the observer keeps its
    // answers in it, t33 K2), and its outputs.
    let own_writes = |n: u32| {
        let written = crate::content_store::ContentStore::open(&content).unwrap();
        written.put_source(format!("store {n}").as_bytes(), &[]).unwrap();
        drop(written);
        std::fs::write(root.join("store.sqlite.upkeep"), format!("frames\t{n}\n")).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("rr_diagnostic.csv"), format!("{n}\n")).unwrap();
    };
    own_writes(0);
    let store = store_beside(&content);
    let cursor = || -> Vec<String> { kept_meta(&store).iter().filter(|(k, _)| k.starts_with("journal:")).filter_map(|(_, v)| text_cursor(v)).collect() };
    let run = |first: bool, n: u32| {
        let mut o = journal_observer(&store);
        o.begin();
        if first {
            o.own_output(&out);
        }
        let marked = o.mark(std::slice::from_ref(&root)).unwrap_or_default();
        let names: Vec<String> = o.listing(&root).unwrap().iter().map(|(n, _)| n.clone()).collect();
        o.digest(&source).unwrap();
        assert!(o.digest(&content).is_some(), "an own file is still answered");
        let kept = [&content, &out.join("rr_diagnostic.csv")].iter().filter(|p| o.entries.contains_key(p.as_path())).count();
        own_writes(n);
        o.flush();
        o.written();
        (o.counts, names, kept, marked)
    };
    let (one, names, kept, _) = run(true, 1);
    assert_eq!(names, ["pkg"], "a listing leaves rerail's own files out");
    assert_eq!(kept, 0, "no answer is kept for an own file");
    // The setup's writes may be named once more.
    std::thread::sleep(std::time::Duration::from_millis(300));
    let (two, _, _, marked) = run(false, 2);
    std::thread::sleep(std::time::Duration::from_millis(300));
    let before = cursor();
    let (third, names, kept, _) = run(false, 3);
    assert_eq!((names.as_slice(), kept), (["pkg".to_owned()].as_slice(), 0));
    if journal_answered(&[&one, &two, &third], "own files") {
        assert_eq!((third.forgotten, third.stats), (0, 0), "the journal's news of own files forgets nothing: {third}");
        assert_eq!(cursor(), before, "and moves no stored cursor: nothing written");
    }
    // A record made in the second run holds: what changed since under its root is rerail's own.
    let mut o = journal_observer(&store);
    o.begin();
    let since = o.changed_since("rec", std::slice::from_ref(&root), &marked);
    if journal_answered(&[&one, &two, &third, &o.counts], "a record's changes") {
        assert_eq!(since, Some(Vec::new()));
    }
    o.flush();
    o.written();
    drop(o);
    let _ = std::fs::remove_dir_all(&base);
}

/// Decision 0284: an output directory is recorded once, and only where a later run's journal reports from: under a
/// root the store has a cursor for. One under a directory already recorded, or outside every such root (phase 0's
/// scratch outputs in the temporary directory), is rerail's own for the process and written nowhere.
#[test]
fn an_output_directory_is_recorded_once_and_only_under_a_watched_root() {
    let base = std::env::temp_dir().join(format!("rerail-own-record-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("root")).unwrap();
    let real = dunce::canonicalize(&base).unwrap();
    let (root, elsewhere) = (real.join("root"), real.join("scratch/out"));
    let store = real.join("store.files");
    journal_observer(&store).written();
    put_meta(&store, cursor_key_text("aa", &root), rusqlite::types::Value::Text("1".into()));
    let recorded = || -> Vec<String> { meta_keys(&store, OWN_OUTPUT) };
    let mut o = journal_observer(&store);
    o.own_output(&root.join("out"));
    o.own_output(&root.join("out/phase-sources"));
    o.own_output(&elsewhere);
    o.written();
    assert_eq!(recorded(), [format!("{OWN_OUTPUT}{}", root.join("out").display())]);
    assert!(o.is_own(&root.join("out/phase-sources/x.csv")) && o.is_own(&elsewhere.join("x.csv")), "all three are own for the process");
    drop(o);
    let mut o = journal_observer(&store);
    o.own_output(&root.join("out"));
    o.written();
    assert_eq!(recorded().len(), 1, "the next process knows it: nothing written again");
    drop(o);
    let _ = std::fs::remove_dir_all(&base);
}

/// An inner root is asked as the stored root above it (`outermost`): answered from that root's cursor, trusted,
/// with no cursor or settling of its own.
#[cfg(target_os = "macos")]
#[test]
fn an_inner_root_is_answered_by_the_root_above_it() {
    let base = std::env::temp_dir().join(format!("rerail-inner-root-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let root = dunce::canonicalize({
        std::fs::create_dir_all(base.join("root/lib")).unwrap();
        base.join("root")
    })
    .unwrap();
    let file = root.join("lib/a.rerail");
    std::fs::write(&file, "x.\n").unwrap();
    aged(&file);
    let store = base.join("store.files");
    let run = |watch: &Path| {
        let mut o = journal_observer(&store);
        o.begin();
        o.watch(&[watch.to_path_buf()]);
        let asked: Vec<(PathBuf, bool)> = o.roots.iter().map(|r| (r.path.clone(), r.trusted)).collect();
        o.digest(&file).unwrap();
        o.flush();
        o.written();
        (asked, o.counts)
    };
    let (_, one) = run(&root);
    std::thread::sleep(std::time::Duration::from_millis(300));
    let (_, two) = run(&root);
    let (asked, inner) = run(&root.join("lib"));
    if journal_answered(&[&one, &two, &inner], "the inner root") {
        assert_eq!(asked, [(root.clone(), true)], "asked as the root above it, and trusted");
    }
    let cursors = meta_keys(&store, "journal:");
    assert!(cursors.iter().all(|k| !k.ends_with("/lib")), "no cursor of its own: {cursors:?}");
    let _ = std::fs::remove_dir_all(&base);
}

/// Asking about rerail's own files watches nothing: no root above them is asked of the journal. Phase 0's
/// scratch corpus, listed, had watched the whole temporary directory, whose news the next run stored.
#[cfg(target_os = "macos")]
#[test]
fn rerails_own_files_watch_nothing() {
    let base = std::env::temp_dir().join(format!("rerail-own-watch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let scratch = base.join("scratch-root");
    std::fs::create_dir_all(&scratch).unwrap();
    std::fs::write(scratch.join("a.py"), "x = 1\n").unwrap();
    let store = base.join("store.files");
    let mut o = journal_observer(&store);
    o.own.push(base.join("scratch"));
    o.begin();
    assert!(o.listing(&scratch).is_some());
    assert_eq!(o.kind(&scratch.join("a.py")), Kind::File);
    assert!(o.digest(&scratch.join("a.py")).is_some() && o.canonical(&scratch).is_some());
    assert!(o.roots.is_empty(), "no root asked: {:?}", o.roots.iter().map(|r| r.path.clone()).collect::<Vec<_>>());
    o.flush();
    drop(o);
    let _ = std::fs::remove_dir_all(&base);
}

/// A first run that watches an inner root before the root around it keeps what it checked under both: each root
/// settles keeping what was checked under it, so the inner one forgot everything checked as the outer one's, and
/// the run after a cold one recorded it all again (`RERAIL_WARM`).
#[cfg(target_os = "macos")]
#[test]
fn a_first_run_keeps_what_it_checked_under_nested_roots() {
    let base = std::env::temp_dir().join(format!("rerail-nested-roots-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let root = dunce::canonicalize({
        std::fs::create_dir_all(base.join("root/lib")).unwrap();
        base.join("root")
    })
    .unwrap();
    let file = root.join("lib/a.rerail");
    std::fs::write(&file, "x.\n").unwrap();
    aged(&file);
    let store = base.join("store.files");
    let other = root.join("b.rerail");
    std::fs::write(&other, "y.\n").unwrap();
    aged(&other);
    let mut o = journal_observer(&store);
    o.begin();
    // Asked under the inner root before the outer one is watched, and under both after.
    o.watch(&[root.join("lib")]);
    assert!(o.canonical(&file).is_some());
    o.watch(std::slice::from_ref(&root));
    o.digest(&file).unwrap();
    o.digest(&other).unwrap();
    assert_eq!(o.roots.iter().map(|r| r.path.clone()).collect::<Vec<_>>(), std::slice::from_ref(&root), "the outer root took the inner one in");
    o.flush();
    o.written();
    drop(o);
    let answers = kept_answers(&store);
    let kept = |p: &Path| -> i64 { i64::from(answers.contains(&stored(p))) };
    let canon = i64::from(kept_forms(&store).contains(&stored(&file)));
    assert_eq!((kept(&file), kept(&other), canon), (1, 1, 1), "the files' answers and the canonical form are kept");
    let cursors = meta_keys(&store, "journal:");
    assert!(cursors.iter().all(|k| !k.ends_with("/lib")), "one cursor, the outer root's: {cursors:?}");
    let _ = std::fs::remove_dir_all(&base);
}

/// The same across runs of one process: an inner root settled at an earlier run's end (phase 0's, before the
/// corpus run) keeps its answers when a later run first asks the root around it.
#[cfg(target_os = "macos")]
#[test]
fn a_first_process_keeps_what_an_earlier_run_checked_under_an_inner_root() {
    let base = std::env::temp_dir().join(format!("rerail-nested-runs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let root = dunce::canonicalize({
        std::fs::create_dir_all(base.join("root/lib")).unwrap();
        base.join("root")
    })
    .unwrap();
    let file = root.join("lib/a.rerail");
    std::fs::write(&file, "x.\n").unwrap();
    aged(&file);
    let store = base.join("store.files");
    let mut o = journal_observer(&store);
    o.begin();
    o.watch(&[root.join("lib")]);
    assert!(o.canonical(&file).is_some());
    o.digest(&file).unwrap();
    o.flush();
    o.begin();
    o.watch(std::slice::from_ref(&root));
    o.flush();
    o.written();
    drop(o);
    let kept = i64::from(kept_answers(&store).contains(&stored(&file)));
    let canon = i64::from(kept_forms(&store).contains(&stored(&file)));
    assert_eq!((kept, canon), (1, 1), "the earlier run's answer and canonical form are kept");
    let _ = std::fs::remove_dir_all(&base);
}

/// A path's canonical form is resolved once: under a root the journal says is unchanged, the next run answers it
/// with no `realpath`, and a symlink retargeted under the root is resolved again. The journal's (FSEvents), as
/// the other journal tests: with none (Linux, no Watchman), nothing is trusted by it.
#[test]
#[cfg(target_os = "macos")]
fn a_canonical_path_is_resolved_again_only_where_the_journal_names_a_change() {
    use std::os::unix::fs::symlink as symlink_file;
    let base = std::env::temp_dir().join(format!("rerail-canonical-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let root = base.join("root");
    std::fs::create_dir_all(root.join("pkg")).unwrap();
    for i in 0..3 {
        std::fs::write(root.join(format!("pkg/m{i}.py")), "x = 1\n").unwrap();
    }
    let link = root.join("pkg/alias.py");
    symlink_file("m0.py", &link).unwrap();
    let store = base.join("store.files");
    let asked = [root.join("pkg/m1.py"), link.clone(), root.join("pkg/../pkg/m2.py")];
    let run = |settle: i64| {
        let mut o = journal_observer(&store);
        o.settle_ns = settle;
        o.begin();
        o.watch(std::slice::from_ref(&root));
        let got: Vec<Option<PathBuf>> = asked.iter().map(|p| o.canonical(p)).collect();
        o.flush();
        (o.counts, got)
    };
    let (first, got) = run(0);
    assert_eq!(first.resolved, 3, "{first}");
    let real = dunce::canonicalize(&root).unwrap();
    assert_eq!(got, vec![Some(real.join("pkg/m1.py")), Some(real.join("pkg/m0.py")), Some(real.join("pkg/m2.py"))]);
    std::thread::sleep(std::time::Duration::from_millis(300));
    let (middle, _) = run(0);
    let (second, same) = run(0);
    assert_eq!(same, got);
    // The path through `..` is resolved each time: it is not a name its directory's listing holds.
    if journal_answered(&[&first, &middle, &second], "nothing changed") {
        assert_eq!((second.resolved, second.trusted), (1, 2), "nothing changed: {second}");
    }
    std::fs::remove_file(&link).unwrap();
    symlink_file("m2.py", &link).unwrap();
    let (third, mut after) = run(0);
    if journal_answered(&[&first, &middle, &second, &third], "a retargeted symlink") {
        assert!(third.resolved >= 2, "the retargeted symlink is resolved again: {third}");
    } else {
        // As a run asks, its settle window on (`journal_answered`): the link is resolved again whatever the
        // journal said.
        after = run(JOURNAL_SETTLE_NS).1;
    }
    assert_eq!(after[1], Some(real.join("pkg/m2.py")), "the retargeted symlink's target");
    let _ = std::fs::remove_dir_all(&base);
}

fn aged(path: &Path) {
    // Old enough to be trusted (past the racy window).
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
    let f = std::fs::File::options().write(true).open(path).or_else(|_| std::fs::File::open(path)).unwrap();
    f.set_modified(old).unwrap();
}

/// A file's digest is reused on its stat, and computed again when it changes; the store keeps it across
/// observers (processes).
#[test]
fn a_files_digest_is_reused_while_its_stat_holds() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("p0.files");
    let file = dir.path().join("a.py");
    std::fs::write(&file, "x = 1\n").unwrap();
    aged(&file);
    // The setup's events delivered before the first run's cursor (decision 0105): a later one names the file, read
    // again (safe, and not what this test is of).
    std::thread::sleep(std::time::Duration::from_millis(300));
    // ctime is not settable: wait out the racy window by recording later than it.
    let mut first = journal_observer(&store);
    first.validate_every = 0;
    let d1 = first.digest(&file).unwrap();
    // Recorded with a time after ctime + RACY_NS, as a later run would.
    for e in first.entries.values_mut() {
        e.recorded_ns += 2 * RACY_NS;
    }
    first.dirty = first.entries.keys().cloned().collect();
    first.flush();
    first.written();
    let mut second = journal_observer(&store);
    second.validate_every = 0;
    assert_eq!(second.digest(&file), Some(d1));
    assert_eq!((second.counts.hashed, second.counts.reused), (0, 1), "unchanged: one stat, no read");
    std::fs::write(&file, "x = 2\n").unwrap();
    // The next run (a run's answers are as its journal found them, decision 0105).
    second.flush();
    let d2 = second.digest(&file).unwrap();
    assert_ne!(d1, d2);
    assert_eq!(second.counts.hashed, 1, "changed: read again");
}

/// A file written just before its recording is not trusted: it may have changed in the same tick.
#[test]
fn a_racy_file_is_read_again() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.py");
    std::fs::write(&file, "x = 1\n").unwrap();
    let mut o = Observer::open(None);
    o.digest(&file).unwrap();
    o.digest(&file).unwrap();
    assert_eq!(o.counts.hashed, 2, "inside the racy window every observation reads");
}

/// An answer recorded inside the racy window is read again at the run's end once the window has passed, and
/// trusted then: the next run (a warm one) neither reads nor records it (Q45). One whose bytes changed is not.
#[test]
fn a_racy_answer_is_settled_at_the_runs_end() {
    let dir = tempfile::tempdir().unwrap();
    let (file, other) = (dir.path().join("a.py"), dir.path().join("b.py"));
    std::fs::write(&file, "x = 1\n").unwrap();
    std::fs::write(&other, "y = 1\n").unwrap();
    let mut o = Observer::open(None);
    o.validate_every = 0;
    let d = o.digest(&file).unwrap();
    o.digest(&other).unwrap();
    o.listing(dir.path()).unwrap();
    assert!(o.entries.values().all(Entry::racy), "written just now: recorded inside the window");
    // Still inside the window: left as it was, and kept for a later end.
    o.settle_racy(now_ns());
    assert!(o.entries[&file].racy());
    assert_eq!(o.racy.len(), 3, "each waits for a later end: {:?}", o.racy);
    // `b.py`'s recorded digest no longer its bytes' (as if written in the recording's tick, its stat kept).
    o.entries.get_mut(&other).unwrap().digest = [7; 32];
    let hashed = o.counts.hashed;
    // Past the window (as a run's end some seconds later is).
    o.settle_racy(now_ns() + 3 * RACY_NS);
    assert_eq!(o.counts.hashed, hashed + 2, "each racy file read again once");
    assert!(!o.entries[&file].racy() && !o.entries[dir.path()].racy(), "the same bytes and listing: trusted");
    assert!(o.entries[&other].racy(), "bytes that differ from the answer: left for the next run to read");
    assert!(o.racy.is_empty());
    assert_eq!(o.entries[&file].digest, d);
}

/// A rewrite that keeps size and mtime is seen by ctime; a validation pass sees what a lying stat hides.
#[test]
fn validation_catches_bytes_changed_under_a_kept_stat() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.py");
    std::fs::write(&file, "x = 1\n").unwrap();
    let mut o = Observer::open(None);
    o.validate_every = 1;
    let d1 = o.digest(&file).unwrap();
    // Pretend the stat is trusted (recorded long after), then change the bytes behind the recorded stat.
    let e = o.entries.get_mut(&file).unwrap();
    e.recorded_ns += 10 * RACY_NS;
    e.digest = [7; 32];
    let d = o.digest(&file).unwrap();
    assert_eq!(d, d1);
    assert_eq!((o.counts.validated, o.counts.mismatched), (1, 1));
}

/// A directory's listing is reused while its stat holds, and listed again when an entry is added.
#[test]
fn a_listing_is_reused_until_an_entry_is_added() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("pkg");
    std::fs::create_dir(&sub).unwrap();
    std::fs::write(sub.join("a.py"), "").unwrap();
    std::fs::create_dir(sub.join("inner")).unwrap();
    let mut o = Observer::open(None);
    let l = o.listing(&sub).unwrap();
    assert_eq!(*l, vec![("a.py".to_owned(), false), ("inner".to_owned(), true)]);
    o.entries.get_mut(&sub).unwrap().recorded_ns += 10 * RACY_NS;
    o.listing(&sub).unwrap();
    assert_eq!((o.counts.listed, o.counts.reused), (1, 1));
    // A new entry changes the directory's mtime and ctime.
    std::thread::sleep(std::time::Duration::from_millis(5));
    std::fs::write(sub.join("b.py"), "").unwrap();
    let l = o.listing(&sub).unwrap();
    assert_eq!(l.len(), 3);
    assert_eq!(o.counts.listed, 2);
}

fn git(dir: &Path, args: &[&str]) {
    // The machine's git settings (a signing key, hooks, a default branch) are not the test's.
    let out = std::process::Command::new("git")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn age_all(dir: &Path) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        if e.file_name() == ".git" {
            continue;
        }
        if e.path().is_dir() {
            age_all(&e.path());
        } else {
            aged(&e.path());
        }
    }
}

/// Git's view: the files it sees (ignored ones left out, untracked ones in, deleted ones out), and a clean
/// file's digest from its blob in another clone, unread.
#[test]
fn git_lists_the_worktree_and_a_clone_reads_no_clean_file() {
    // Windows names its system `gitattributes` only through the Git setup a host installs (`git_access::setup`),
    // and with none no file is proven clean by its stat: installed here from the answer the host asks for.
    #[cfg(windows)]
    {
        let system = std::process::Command::new("git").args(["var", "GIT_CONFIG_SYSTEM"]).env_remove("GIT_CONFIG_SYSTEM").env_remove("GIT_CONFIG_NOSYSTEM").output().unwrap();
        assert!(system.status.success(), "git var GIT_CONFIG_SYSTEM");
        crate::git_access::setup::install(std::sync::Arc::new(crate::git_access::setup::GitSetup::from_report(Some(&String::from_utf8_lossy(&system.stdout)), None)));
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(repo.join("pkg")).unwrap();
    git(&repo, &["init", "--initial-branch=main"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("a.py"), "x = 1\n").unwrap();
    std::fs::write(repo.join("pkg/b.rerail"), "r(1).\n").unwrap();
    std::fs::write(repo.join("gone.py"), "").unwrap();
    std::fs::write(repo.join(".gitignore"), "*.log\n").unwrap();
    git(&repo, &["add", "--", "a.py", "pkg/b.rerail", "gone.py", ".gitignore"]);
    git(&repo, &["commit", "-m", "one"]);
    std::fs::remove_file(repo.join("gone.py")).unwrap();
    std::fs::write(repo.join("new.py"), "").unwrap();
    std::fs::write(repo.join("noise.log"), "").unwrap();
    let g = crate::git_workspace::discover(&repo).unwrap();
    assert_eq!(g.worktree_files(), vec![".gitignore", "a.py", "new.py", "pkg/b.rerail"]);

    // A clone whose index stats are settled (mtimes before the index): its clean files are proven by stat.
    let clone = dir.path().join("clone");
    git(dir.path(), &["clone", "-q", repo.to_str().unwrap(), clone.to_str().unwrap()]);
    age_all(&repo);
    age_all(&clone);
    // The index is written after a later tick than the files' ctimes: its entries are not racy.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    git(&repo, &["update-index", "--ignore-missing", "--refresh"]);
    git(&clone, &["update-index", "--refresh"]);
    for root in [&repo, &clone] {
        assert!(crate::git_workspace::discover(root).unwrap().clean_blob_unread("a.py").is_some(), "{}: a.py proven clean by its stat", root.display());
    }
    let store = dir.path().join("p0.files");
    let mut o = journal_observer(&store);
    o.validate_every = 0;
    o.git(&repo).unwrap();
    let d = o.digest(&repo.join("a.py")).unwrap();
    assert_eq!((o.counts.hashed, o.counts.git), (1, 0), "first sight of the blob: read once, remembered");
    o.flush();
    o.written();
    let mut fresh = journal_observer(&store);
    fresh.validate_every = 0;
    fresh.git(&clone).unwrap();
    assert_eq!(fresh.digest(&clone.join("a.py")), Some(d));
    assert_eq!((fresh.counts.hashed, fresh.counts.git), (0, 1), "the clone's clean a.py: its blob's digest, unread");
    // An edited file is not clean: hashed.
    std::fs::write(clone.join("a.py"), "x = 2\n").unwrap();
    fresh.begin();
    fresh.git(&clone).unwrap();
    assert_ne!(fresh.digest(&clone.join("a.py")), Some(d));
    assert_eq!(fresh.counts.hashed, 1);
}

/// `forget` takes a path named and what is under it, and its directory's own answer: a range of the sorted
/// answers, which leaves a sibling whose name only starts the same (`b.txt` beside `b`).
#[test]
fn forget_takes_what_is_under_a_path_and_no_sibling() {
    let mut o = Observer::open(None);
    let stat = Stat::default();
    let keep = ["/r", "/r/a", "/r/a.txt", "/r/a0/x", "/r/b/c", "/q/a/x"];
    let forgotten = ["/r/a/x", "/r/a/y/z", "/r/b"];
    for p in keep.iter().chain(&forgotten) {
        o.entries.insert(PathBuf::from(p), Entry { kind: Kind::File, stat, digest: [0; 32], listing: None, recorded_ns: 0 });
        o.canon.insert(PathBuf::from(p), PathBuf::from(p));
    }
    // `/r/a` named changed: it and what is under it go, and its directory `/r`'s answer; `/r/b` its own.
    o.forget(Path::new("/r"), &[PathBuf::from("/r/a"), PathBuf::from("/r/b/c")], None, "");
    let left: Vec<&str> = o.entries.keys().map(|k| k.to_str().unwrap()).collect();
    assert_eq!(left, ["/q/a/x", "/r/a.txt", "/r/a0/x"]);
    let canon: Vec<&str> = o.canon.keys().map(|k| k.to_str().unwrap()).collect();
    assert_eq!(canon, ["/q/a/x", "/r", "/r/a.txt", "/r/a0/x", "/r/b"], "a canonical form goes only under a path named");
    assert_eq!(o.counts.forgotten, 6);
}

/// A root the journal names nothing under keeps its stored cursor, until the cursor is `CURSOR_AGE` events
/// behind: it is then stored again, once.
#[test]
fn an_aged_cursor_is_stored_again() {
    let base = std::env::temp_dir().join(format!("rerail-aged-cursor-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let store = base.join("store.files");
    let (root, volume) = (Path::new("/r"), [7u8; 16]);
    let key = cursor_key(root, volume);
    let stored = || -> Option<String> { kept_cursor(&store, &key) };
    let mut o = journal_observer(&store);
    o.db.as_mut().unwrap().stored.insert(key.clone(), "1000".into());
    o.forget(root, &[], Some(volume), &(1000 + CURSOR_AGE - 1).to_string());
    o.written();
    assert_eq!(stored(), None, "not yet aged: nothing written");
    o.forget(root, &[], Some(volume), &(1000 + CURSOR_AGE).to_string());
    o.written();
    assert_eq!(stored(), Some((1000 + CURSOR_AGE).to_string()));
    o.forget(root, &[], Some(volume), &(1000 + CURSOR_AGE + 5).to_string());
    o.written();
    assert_eq!(stored(), Some((1000 + CURSOR_AGE).to_string()), "stored again once, then aged from there");
    drop(o);
    let _ = std::fs::remove_dir_all(&base);
}

/// The observer's database names the user's files, their stats and digests: private to the user, as the store
/// is (docs/privacy.md). It was made with the umask's mode (`0644`), readable by every user of the machine.
#[cfg(unix)]
#[test]
fn the_observers_database_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let base = std::env::temp_dir().join(format!("rerail-observer-private-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let store = base.join("store.files");
    let o = journal_observer(&store);
    o.written();
    let mode = |p: &Path| std::fs::metadata(p).map(|m| m.permissions().mode() & 0o777).ok();
    assert_eq!(mode(&store), Some(0o600));
    let mut wal = store.as_os_str().to_owned();
    wal.push("-wal");
    assert!(matches!(mode(Path::new(&wal)), None | Some(0o600)), "its WAL takes its mode");
    assert!(matches!(mode(&cookie_dir(&store)), None | Some(0o700)));
    drop(o);
    let _ = std::fs::remove_dir_all(&base);
}

/// News that forgets nothing (another program's file under a watched root) keeps the stored cursor too, until it
/// ages or its replay names `CURSOR_PATHS` paths: a warm run wrote the moved cursor whenever anything changed
/// under a root, though nothing it kept had (`RERAIL_WARM`). News that forgets an answer stores it at once.
#[test]
fn news_that_forgets_nothing_keeps_the_stored_cursor() {
    let base = std::env::temp_dir().join(format!("rerail-unrelated-news-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let store = base.join("store.files");
    let (root, volume) = (Path::new("/r"), [7u8; 16]);
    let key = cursor_key(root, volume);
    let stored = || -> Option<String> { kept_cursor(&store, &key) };
    let mut o = journal_observer(&store);
    o.db.as_mut().unwrap().stored.insert(key.clone(), "1000".into());
    // The root's part of the store read first, as `watch` reads it.
    o.ensure(root);
    o.entries.insert(PathBuf::from("/r/kept.py"), Entry { kind: Kind::File, stat: Stat::default(), digest: [0; 32], listing: None, recorded_ns: 0 });
    o.forget(root, &[PathBuf::from("/r/other/x.lock")], Some(volume), "1010");
    o.written();
    assert_eq!(stored(), None, "nothing forgotten: nothing written");
    let many: Vec<PathBuf> = (0..CURSOR_PATHS).map(|i| PathBuf::from(format!("/r/other/{i}"))).collect();
    o.forget(root, &many, Some(volume), "1020");
    o.written();
    assert_eq!(stored(), Some("1020".to_owned()), "a replay of that many paths: stored again");
    o.forget(root, &[PathBuf::from("/r/kept.py")], Some(volume), "1030");
    o.written();
    assert_eq!(stored(), Some("1030".to_owned()), "an answer forgotten: stored with the delete");
    drop(o);
    let _ = std::fs::remove_dir_all(&base);
}

/// A record refreshed for its aged cursors keeps everything but its first list, in memory and in the store.
#[test]
fn a_refreshed_record_moves_only_its_cursors() {
    let base = std::env::temp_dir().join(format!("rerail-refreshed-record-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let store = base.join("store.files");
    let mut o = journal_observer(&store);
    let record = |cursors: &[&str]| encode_lists(&[cursors, &["/r", "/r/.git"], &["/r/.git/HEAD"]]);
    o.put_run_record("git-files:/r", &record(&["1", "1"]), b"2\na.py");
    o.refresh_record("git-files:/r", &["9".into(), "9".into()]);
    o.written();
    let want = (record(&["9", "9"]), b"2\na.py".to_vec());
    assert_eq!(o.run_record("git-files:/r"), Some(want.clone()));
    drop(o);
    assert_eq!(journal_observer(&store).run_record("git-files:/r"), Some(want));
    let _ = std::fs::remove_dir_all(&base);
}

/// The store is read a part of the tree at a time, as it is first asked (`ensure`): an answer under one directory
/// reads that directory's and no other's; what memory holds wins over the store; and what is forgotten before its
/// part is read is not read back.
#[test]
fn the_store_is_read_a_part_at_a_time() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("p0.files");
    let entry = |size| Entry { kind: Kind::File, stat: Stat { size, ..Stat::default() }, digest: [0; 32], listing: None, recorded_ns: 0 };
    let paths = ["/x/a/f", "/x/a/g", "/y/b/f", "/y/b/g"].map(PathBuf::from);
    let mut first = journal_observer(&store);
    for p in &paths {
        first.entries.insert(p.clone(), entry(1));
        first.dirty.push(p.clone());
    }
    first.flush();
    first.written();
    drop(first);
    let mut o = journal_observer(&store);
    assert!(o.entries.is_empty(), "nothing read at open");
    assert_eq!(o.known(&paths[0]).map(|e| e.stat.size), Some(1));
    let held: Vec<&Path> = o.entries.keys().map(PathBuf::as_path).collect();
    assert_eq!(held, [paths[0].as_path(), paths[1].as_path()], "only /x/a read");
    // Memory ahead of the store: a part read later keeps what memory holds.
    o.entries.insert(paths[3].clone(), entry(9));
    // Forgotten before /y/b is read: the store's row is deleted first, not read back.
    o.forget(Path::new("/y"), &[paths[2].clone()], None, "");
    assert!(o.known(&paths[2]).is_none(), "forgotten, not read back");
    assert_eq!(o.known(&paths[3]).map(|e| e.stat.size), Some(9), "memory wins");
}

/// `collect` drops what no run asks any more, and keeps the rest.
#[test]
fn realpaths_a_directory_at_a_time_are_each_files_own() {
    let given = std::env::temp_dir().join(format!("rerail-realpaths-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&given);
    std::fs::create_dir_all(given.join("d/e")).unwrap();
    std::fs::write(given.join("d/a.py"), "").unwrap();
    std::fs::write(given.join("d/e/b.py"), "").unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(given.join("d/e/b.py"), given.join("d/link.py")).unwrap();
        std::os::unix::fs::symlink(given.join("d/e"), given.join("d/dirlink")).unwrap();
    }
    let asked: Vec<PathBuf> = ["d/a.py", "d/e/b.py", "d/link.py", "d/dirlink/b.py", "d/missing.py", "d/A.py", "d/e/../a.py"].iter().map(|p| given.join(p)).collect();
    let refs: Vec<&Path> = asked.iter().map(PathBuf::as_path).collect();
    let one_by_one: Vec<Option<PathBuf>> = asked.iter().map(|p| dunce::canonicalize(p).ok()).collect();
    assert_eq!(realpaths(&refs), one_by_one, "as `realpath` gives each: symlinks, a missing file, another case, `..`");
    let _ = std::fs::remove_dir_all(&given);
}

#[test]
fn collection_drops_what_no_run_asks_any_more() {
    let given = std::env::temp_dir().join(format!("rerail-observer-collect-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&given);
    std::fs::create_dir_all(given.join("root/inner")).unwrap();
    let real = dunce::canonicalize(&given).unwrap();
    let store = real.join("store.files");
    journal_observer(&store).written();
    let (root, inner, gone) = (real.join("root"), real.join("root/inner"), real.join("gone"));
    let dotted = real.join("root/../root");
    let s = |p: &Path| stored(p);
    let db = Connection::open(&store).unwrap();
    let meta = |root: &Path, cursor: &str| put_meta(&store, cursor_key_text("aa", root), rusqlite::types::Value::Text(cursor.to_owned()));
    meta(&root, "100");
    meta(&inner, "50");
    meta(&gone, "10");
    meta(&dotted, "5");
    // Another name of the same directory, with an older cursor (`/var/…` beside `/private/var/…` on macOS).
    let alias = (given != real).then(|| given.join("root"));
    if let Some(a) = &alias {
        meta(a, "20");
    }
    // Two names of one directory one run asked, with the same listing cursor, are both kept; a third name with an
    // older generation is dropped.
    std::fs::create_dir_all(real.join("listed")).unwrap();
    let (same, older) = (real.join("same"), real.join("older"));
    #[cfg(unix)]
    for link in [&same, &older] {
        std::os::unix::fs::symlink(real.join("listed"), link).unwrap();
    }
    meta(&real.join("listed"), "L3");
    meta(&same, "L3");
    meta(&older, "L1");
    let answer = |p: &Path| put_answer(&store, p, 0);
    answer(&root.join("a.py"));
    answer(&real.join("elsewhere/b.py"));
    answer(&gone.join("c.py"));
    if let Some(a) = &alias {
        answer(&a.join("a.py"));
    }
    // A root a run first asked has no cursor stored until the run's end: what the run recorded under it meanwhile
    // is young, and kept (it was collected as the run ended, and the next run recorded it all again).
    let fresh = real.join("fresh");
    put_answer(&store, &fresh.join("d.py"), now_ns());
    put_form(&store, &fresh, &fresh, now_ns());
    let (live, dead) = ([1u8; 32], [2u8; 32]);
    let record = |key: &str, consulted: &str| db.execute("INSERT INTO run_record VALUES (?1, ?2, x'')", params![key, consulted]).unwrap();
    // A cutoff's consulted list as the engine keeps it (`reuse::encode_consulted`): kind, path, digest by turns.
    let cutoff = |key: &str, flat: &[&str]| db.execute("INSERT INTO run_record VALUES (?1, ?2, x'')", params![key, encode_lists(&[flat])]).unwrap();
    put_form(&store, &root.join("a.py"), &root.join("a.py"), 0);
    put_form(&store, &real.join("elsewhere"), &real.join("elsewhere"), 0);
    // A canonical form's record from before they were kept facts.
    record(&format!("{CANONICAL}{}", s(&root.join("old.py"))), "");
    cutoff("cutoff:kept", &["rule", "x.rerail", "ab", "plan", "", &hex(&dead), "plan", "", &hex(&live)]);
    cutoff("cutoff:dead", &["plan", "", &hex(&dead)]);
    cutoff(&"ab".repeat(32), &["rule", "x.rerail", "ab"]);
    // The constants' and the cone's rounds' records go with their plans too; one naming none is from before.
    cutoff("constants:k", &[]);
    cutoff("constants:live", &["doc", "a.py", "ab", "plan", "", &hex(&live)]);
    cutoff("constants:dead", &["doc", "a.py", "ab", "plan", "", &hex(&dead)]);
    let rounds = |key: &str, plan: &[u8; 32]| db.execute("INSERT INTO run_record VALUES (?1, ?2, ?3)", params![key, encode_lists(&[&["1", "2"][..], &["/r", "/s"], &["plan", "", &hex(plan)]]), 7u64.to_le_bytes().to_vec()]).unwrap();
    rounds("cone-rounds:live", &live);
    rounds("cone-rounds:dead", &dead);
    // An output directory still there is kept; one removed goes (decision 0284).
    put_meta(&store, format!("{OWN_OUTPUT}{}", s(&inner)), rusqlite::types::Value::Integer(1));
    put_meta(&store, format!("{OWN_OUTPUT}{}", s(&gone)), rusqlite::types::Value::Integer(1));
    drop(db);
    let swept = collect(&store, |plan| *plan == live).unwrap();
    let db = Connection::open(&store).unwrap();
    let keys = |sql: &str| -> Vec<String> { db.prepare(sql).unwrap().query_map([], |r| r.get(0)).unwrap().flatten().collect() };
    let roots = meta_keys(&store, "journal:");
    let mut want = vec![cursor_key_text("aa", &real.join("listed")), cursor_key_text("aa", &root)];
    // `same` is the listed directory's second name where a link made it (Unix); elsewhere it names nothing, and
    // goes as `gone` does.
    if cfg!(unix) {
        want.push(cursor_key_text("aa", &same));
    }
    want.sort();
    assert_eq!(roots, want, "root: inner is under it, gone is gone, dotted and the alias are other names with older cursors; listed and same: one directory's two names of one run");
    let answers = kept_answers(&store);
    assert_eq!(answers, [s(&fresh.join("d.py")), s(&root.join("a.py"))]);
    let mut records = keys("SELECT key FROM run_record");
    records.sort();
    assert_eq!(records, ["cone-rounds:live", "constants:live", "cutoff:kept"]);
    assert_eq!(kept_forms(&store), [s(&fresh), s(&root.join("a.py"))]);
    let n = usize::from(alias.is_some());
    assert_eq!(meta_keys(&store, OWN_OUTPUT), [format!("{OWN_OUTPUT}{}", s(&inner))]);
    // Without the link (not Unix), `same` names nothing and its cursor goes too.
    let unlinked = usize::from(!cfg!(unix));
    assert_eq!(swept, Swept { cursors: 4 + n + unlinked, answers: 3 + n, records: 6, outputs: 1, changes: 0 });
    assert_eq!(std::fs::metadata(real.join("store.files-wal")).map_or(0, |m| m.len()), 0, "the WAL folded in");
    drop(db);
    let _ = std::fs::remove_dir_all(&given);
}

#[test]
fn listings_round_trip_through_the_store_encoding() {
    let l = vec![("a".to_owned(), true), ("b.py".to_owned(), false)];
    assert_eq!(decode_listing(&encode_listing(&l).unwrap()).unwrap(), l);
    assert_eq!(decode_listing(&encode_listing(&[]).unwrap()).unwrap(), Vec::<(String, bool)>::new());
    assert!(encode_listing(&[("a\nb".to_owned(), false)]).is_none());
}

/// What `changed_since` names for `paths`: each under both names of its root, as given and canonical, where they
/// differ (a temporary directory under macOS's `/var`), sorted.
fn named(paths: &[&PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = paths.iter().flat_map(|p| spellings(p)).collect();
    out.sort();
    out.dedup();
    out
}

/// A tree for the listing tests (decision 0400): `root/pkg/m0.py` … `m4.py`, each past the racy window.
#[cfg(any(unix, windows))]
fn listing_tree(name: &str) -> (PathBuf, PathBuf, Vec<PathBuf>) {
    let base = std::env::temp_dir().join(format!("rerail-listing-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let root = base.join("root");
    std::fs::create_dir_all(root.join("pkg")).unwrap();
    let files: Vec<PathBuf> = (0..5).map(|i| root.join(format!("pkg/m{i}.py"))).collect();
    for f in &files {
        std::fs::write(f, "x = 1\n").unwrap();
        aged(f);
    }
    aged_dir(&root.join("pkg"));
    // Setting a time moves the ctime to now: past the racy window of that, the first run's answers hold.
    std::thread::sleep(std::time::Duration::from_nanos(RACY_NS as u64 + 50_000_000));
    (base, root, files)
}

/// A directory's times set back past the racy window, as `aged` does a file's mtime (its ctime is the setting's).
#[cfg(any(unix, windows))]
fn aged_dir(dir: &Path) {
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
    #[cfg(unix)]
    let opened = std::fs::File::open(dir).unwrap();
    // Windows opens a directory only with backup semantics, and sets its times with FILE_WRITE_ATTRIBUTES.
    #[cfg(windows)]
    let opened = {
        use std::os::windows::fs::OpenOptionsExt as _;
        const FILE_WRITE_ATTRIBUTES: u32 = 0x100;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        std::fs::OpenOptions::new().access_mode(FILE_WRITE_ATTRIBUTES).custom_flags(FILE_FLAG_BACKUP_SEMANTICS).open(dir).unwrap()
    };
    opened.set_modified(old).unwrap();
}

/// One run of a new process's observer over `root`: its listing of `pkg`, each file's digest, its counts.
#[cfg(any(unix, windows))]
fn listing_run(store: &Path, root: &Path, files: &[PathBuf]) -> (Counts, Option<Listing>, Vec<Option<[u8; 32]>>) {
    let mut o = Observer::open(Some(store));
    o.begin();
    o.watch(&[root.to_path_buf()]);
    let listing = o.listing(&root.join("pkg"));
    let digests = files.iter().map(|f| o.digest(f)).collect();
    o.flush();
    o.written();
    (o.counts, listing, digests)
}

/// R1: a second run over an unchanged small root makes no `stat` of a file (one of the root itself, whose answer
/// no listing of its own holds): it lists the two directories holding answers and trusts every answer.
#[cfg(any(unix, windows))]
#[test]
fn a_small_root_is_checked_by_listings_not_stats() {
    let (base, root, files) = listing_tree("unchanged");
    let store = base.join("store.files");
    let (first, listing, digests) = listing_run(&store, &root, &files);
    assert!(first.stats >= 6 && first.trusted == 0, "the first run checks every path: {first}");
    // Past the racy window of the first run's own recording (the directory's answer too).
    std::thread::sleep(std::time::Duration::from_nanos(RACY_NS as u64 + 50_000_000));
    let (second, again, same) = listing_run(&store, &root, &files);
    assert_eq!((second.stats, second.trusted, second.checked_dirs, second.checked_named, second.hashed), (1, 6, 2, 0, 0), "nothing changed: no stat ({second})");
    assert_eq!((again, same), (listing, digests));
    let _ = std::fs::remove_dir_all(&base);
}

/// R2: an edit is named and only it, also one that keeps the size and mtime (its ctime moves); a removed file is
/// named, and a new one, as entries of their directory (0433): not the directory, whose naming would cover its
/// siblings' answers too, and not the siblings.
#[cfg(any(unix, windows))]
#[test]
fn a_listing_names_an_edited_file_and_no_other() {
    let (base, root, files) = listing_tree("edited");
    let store = base.join("store.files");
    listing_run(&store, &root, &files);
    std::thread::sleep(std::time::Duration::from_nanos(RACY_NS as u64 + 50_000_000));
    let (_, listing, digests) = listing_run(&store, &root, &files);
    // The same size and mtime, other bytes: only the ctime says so.
    let mtime = std::fs::metadata(&files[2]).unwrap().modified().unwrap();
    std::fs::write(&files[2], "x = 2\n").unwrap();
    std::fs::File::options().write(true).open(&files[2]).unwrap().set_modified(mtime).unwrap();
    std::thread::sleep(std::time::Duration::from_nanos(RACY_NS as u64 + 50_000_000));
    let (third, same_listing, after) = listing_run(&store, &root, &files);
    assert_eq!((third.checked_named, third.hashed, third.stats), (1, 1, 2), "the edited file alone is named and read again: {third}");
    assert_ne!(after[2], digests[2]);
    assert_eq!([after[0], after[1], after[3], after[4]], [digests[0], digests[1], digests[3], digests[4]]);
    assert_eq!(same_listing, listing);
    std::fs::remove_file(&files[4]).unwrap();
    std::fs::write(root.join("pkg/new.py"), "y = 1\n").unwrap();
    std::thread::sleep(std::time::Duration::from_nanos(RACY_NS as u64 + 50_000_000));
    let (fourth, listing, after) = listing_run(&store, &root, &files);
    // The removed file and the new one; the others hold and are not read.
    assert_eq!((fourth.checked_named, fourth.hashed), (2, 0), "{fourth}");
    assert_eq!(fourth.forgotten, 2, "the removed file's answer and the directory's own: {fourth}");
    assert_eq!(after[4], None);
    let names: Vec<String> = listing.unwrap().iter().map(|(n, _)| n.clone()).collect();
    assert_eq!(names, ["m0.py", "m1.py", "m2.py", "m3.py", "new.py"]);
    let _ = std::fs::remove_dir_all(&base);
}

/// An answer recorded inside the racy window (a file written just before a run) is checked by its bytes at the
/// next run's listings: named only where they changed (0433). The journal names only what changed; the listings
/// named every racy answer, and a header written before a run asked again for the unit that read it.
#[cfg(any(unix, windows))]
#[test]
fn a_racy_answer_with_its_bytes_unchanged_is_not_named() {
    let (base, root, files) = listing_tree("racy");
    let store = base.join("store.files");
    listing_run(&store, &root, &files);
    std::thread::sleep(std::time::Duration::from_nanos(RACY_NS as u64 + 50_000_000));
    listing_run(&store, &root, &files);
    // Edited, then recorded at once by a run: inside the racy window.
    std::fs::write(&files[1], "x = 22\n").unwrap();
    let (_, _, edited) = listing_run(&store, &root, &files);
    let (next, _, again) = listing_run(&store, &root, &files);
    assert_eq!((next.checked_named, next.validated), (0, 1), "the racy answer read again, its bytes as they were: {next}");
    assert_eq!(again, edited);
    let _ = std::fs::remove_dir_all(&base);
}

/// R3: an answer that a path is missing holds while its directory is gone; when a directory is removed, each
/// answer under it is named, and none is read back.
#[cfg(any(unix, windows))]
#[test]
fn a_directory_that_cannot_be_listed_names_what_it_held() {
    let (base, root, mut files) = listing_tree("gone");
    files.push(root.join("absent/x.py"));
    let store = base.join("store.files");
    listing_run(&store, &root, &files);
    std::thread::sleep(std::time::Duration::from_nanos(RACY_NS as u64 + 50_000_000));
    let (second, _, digests) = listing_run(&store, &root, &files);
    assert_eq!((second.checked_named, second.stats), (0, 1), "a missing path under a missing directory holds: {second}");
    assert_eq!(digests[5], None);
    std::fs::remove_dir_all(root.join("pkg")).unwrap();
    let (third, listing, after) = listing_run(&store, &root, &files);
    assert!(third.checked_named >= 6, "the directory and each file under it: {third}");
    assert_eq!((listing, after.iter().flatten().count()), (None, 0));
    let _ = std::fs::remove_dir_all(&base);
}

/// R4: an answer recorded inside the racy window is named by the listing: its file may have changed in the tick
/// it was recorded in. The listing's own stat of the file, with the answer recorded just inside the window and
/// just past it (a run timed against the window would depend on the host's load).
#[cfg(any(unix, windows))]
#[test]
fn a_racy_answer_is_named_by_the_listing() {
    let (base, root, files) = listing_tree("racy-named");
    std::fs::write(&files[0], "x = 3\n").unwrap();
    let name = files[0].file_name().unwrap().to_owned();
    let listed = listing::entries(&root.join("pkg")).unwrap().into_iter().find(|(n, _)| *n == name).and_then(|(_, l)| l);
    let Some((Kind::File, stat)) = listed else { panic!("the file is listed with its stat: {listed:?}") };
    let changed = stat.mtime_ns.max(stat.ctime_ns);
    let answer = |recorded_ns: i64| Entry { kind: Kind::File, stat, digest: [0; 32], listing: None, recorded_ns };
    assert!(!listed_holds(&answer(changed + RACY_NS / 2), listed), "recorded inside the window: named");
    assert!(listed_holds(&answer(changed + RACY_NS + 1), listed), "recorded past it: holds");
    let _ = std::fs::remove_dir_all(&base);
}

/// R5: a listing's stat of an entry is `std::fs::metadata`'s: a file's (a compressed one too, on macOS), a
/// directory's but for its size; a symbolic link is left to be `stat`ed on its own.
#[cfg(any(unix, windows))]
#[test]
fn the_bulk_listing_gives_what_stat_gives() {
    let base = std::env::temp_dir().join(format!("rerail-listing-stat-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("dir/sub")).unwrap();
    std::fs::write(base.join("dir/a.py"), "a = 1\n").unwrap();
    std::fs::write(base.join("dir/empty"), "").unwrap();
    // A symbolic link needs a privilege on Windows: made on Unix alone.
    #[cfg(unix)]
    std::os::unix::fs::symlink("a.py", base.join("dir/link")).unwrap();
    let big = base.join("big.txt");
    std::fs::write(&big, "abcdefgh".repeat(16 * 1024)).unwrap();
    // A file the file system keeps compressed (APFS): its size is its bytes', not its blocks'.
    #[cfg(target_os = "macos")]
    {
        let made = std::process::Command::new("ditto").arg("--hfsCompression").arg(&big).arg(base.join("dir/compressed.txt")).status();
        assert!(made.is_ok_and(|s| s.success()), "ditto made the compressed copy");
    }
    let listed = listing::entries(&base.join("dir")).unwrap();
    let mut names: Vec<String> = listed.iter().map(|(n, _)| n.to_string_lossy().into_owned()).collect();
    names.sort();
    let mut want = if cfg!(unix) { vec!["a.py", "empty", "link", "sub"] } else { vec!["a.py", "empty", "sub"] };
    if cfg!(target_os = "macos") {
        want.insert(1, "compressed.txt");
    }
    assert_eq!(names, want);
    for (name, got) in &listed {
        let path = base.join("dir").join(name);
        let name = name.to_string_lossy();
        let m = std::fs::symlink_metadata(&path).unwrap();
        if m.file_type().is_symlink() {
            assert_eq!(*got, None, "{name}: a link is stat'ed alone");
            continue;
        }
        let (kind, stat) = got.unwrap_or_else(|| panic!("{name}: no stat from the listing"));
        let want = kind_and_stat(&path).unwrap().1;
        if m.is_dir() {
            assert_eq!((kind, stat), (Kind::Dir, Stat { size: 0, ..want }), "{name}");
        } else {
            assert_eq!((kind, stat), (Kind::File, want), "{name}");
        }
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// R6: a run record made at a root's cursor of the observer's own journal is told of what changed after it, over
/// later runs, and of nothing before it.
#[cfg(any(unix, windows))]
#[test]
fn a_run_record_is_answered_from_the_observers_own_journal() {
    let (base, root, files) = listing_tree("record");
    let store = base.join("store.files");
    let run = |edit: Option<&Path>, asked: &[String]| -> (String, Vec<Option<Vec<PathBuf>>>) {
        if let Some(f) = edit {
            std::fs::write(f, "x = 9\n").unwrap();
            aged(f);
            // Past the racy window of the edit's ctime: the answer this run records holds for the next.
            std::thread::sleep(std::time::Duration::from_nanos(RACY_NS as u64 + 50_000_000));
        }
        let mut o = Observer::open(Some(&store));
        o.begin();
        let cursor = o.mark(std::slice::from_ref(&root)).unwrap().remove(0);
        for f in &files {
            o.digest(f);
        }
        let told = asked.iter().map(|c| o.changed_since("test-record", std::slice::from_ref(&root), std::slice::from_ref(c)).map(|mut v| { v.sort(); v })).collect();
        o.flush();
        o.written();
        (cursor, told)
    };
    let (_, _) = run(None, &[]);
    std::thread::sleep(std::time::Duration::from_nanos(RACY_NS as u64 + 50_000_000));
    let (c1, _) = run(None, &[]);
    assert!(listing_cursor(&c1).is_some(), "the root's cursor is the observer's own: {c1}");
    let (c2, told) = run(Some(&files[1]), std::slice::from_ref(&c1));
    assert_eq!(told, [Some(named(&[&files[1]]))]);
    let (_, told) = run(Some(&files[3]), &[c1.clone(), c2.clone()]);
    assert_eq!(told, [Some(named(&[&files[1], &files[3]])), Some(named(&[&files[3]]))]);
    let (c4, told) = run(None, &[c2]);
    assert_eq!(told, [Some(named(&[&files[3]]))], "nothing changed since: the same answer");
    let (_, told) = run(None, &[c4]);
    assert_eq!(told, [Some(vec![])]);
    let _ = std::fs::remove_dir_all(&base);
}

/// R8: the observer's journal tells a reader what changed, as FSEvents' replay did: a file added to a listed
/// directory is named, not the directory (whose answer, its listing, is forgotten all the same; a reader takes a
/// path named with all under it, as the cone's `still` does); and a file written just before one run and not since
/// is not named to the next while its answer is still inside the racy window, its bytes being the answer's.
#[cfg(any(unix, windows))]
#[test]
fn the_journal_names_an_added_entry_and_not_a_racy_answer_that_holds() {
    let (base, root, files) = listing_tree("told");
    let store = base.join("store.files");
    let pkg = root.join("pkg");
    let run = |asked: &[String]| -> (String, Vec<Option<Vec<PathBuf>>>, Counts) {
        let mut o = Observer::open(Some(&store));
        o.begin();
        let cursor = o.mark(std::slice::from_ref(&root)).unwrap().remove(0);
        o.listing(&pkg);
        for f in &files {
            o.digest(f);
        }
        let told = asked.iter().map(|c| o.changed_since("test-told", std::slice::from_ref(&root), std::slice::from_ref(c)).map(|mut v| { v.sort(); v })).collect();
        o.flush();
        o.written();
        (cursor, told, o.counts)
    };
    run(&[]);
    std::thread::sleep(std::time::Duration::from_nanos(RACY_NS as u64 + 50_000_000));
    let (c1, _, _) = run(&[]);
    std::fs::write(pkg.join("new.py"), "y = 1\n").unwrap();
    let (c2, told, counts) = run(std::slice::from_ref(&c1));
    assert_eq!(told, [Some(named(&[&pkg.join("new.py")]))], "the entry added, not its directory");
    assert_eq!(counts.checked_named, 1, "the directory's answer is forgotten: {counts}");
    // Written now: the next run records its answer inside the window; the one after reads it again and names
    // nothing, its bytes unchanged.
    std::fs::write(&files[1], "x = 7\n").unwrap();
    let (c3, told, _) = run(std::slice::from_ref(&c2));
    assert_eq!(told, [Some(named(&[&files[1]]))]);
    let (_, told, counts) = run(std::slice::from_ref(&c3));
    assert_eq!(told, [Some(vec![])], "the racy answer's bytes are still the file's: {counts}");
    let _ = std::fs::remove_dir_all(&base);
}
