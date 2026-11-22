use super::super::*;

/// A test database's path, in a directory of its own that goes with it: the store's side files (WAL, locks, the
/// files database) too. Bound before the stores opened on it, so it is dropped after them.
struct TestPath {
    path: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

impl std::ops::Deref for TestPath {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.path
    }
}

impl AsRef<std::path::Path> for TestPath {
    fn as_ref(&self) -> &std::path::Path {
        &self.path
    }
}

fn path(label: &str) -> TestPath {
    let dir = tempfile::Builder::new().prefix(&format!("rerail-atoms-{label}-")).tempdir().unwrap();
    TestPath { path: dir.path().join("store.sqlite"), _dir: dir }
}

/// The store's atoms, by id, as its blocks hold them on its rows (read on a connection of its own): what any process
/// opening it reads.
fn stored(path: &std::path::Path) -> Vec<String> {
    super::stored::every_stored(&rusqlite::Connection::open(path).unwrap()).unwrap()
}

/// Appends `strings` at the store's end, as another process (giving out provisional ids, decision 0160) would.
fn another(path: &std::path::Path, strings: &[&str]) {
    let strings: Vec<String> = strings.iter().map(|s| (*s).to_owned()).collect();
    super::stored::append_as_another(&rusqlite::Connection::open(path).unwrap(), &strings).unwrap();
}

#[test]
fn atoms_are_dense_stable_and_survive_reopening() {
    let path = path("dense");
    let store = ContentStore::open(&path).unwrap();
    assert_eq!(store.atom("").unwrap(), Some(0));
    assert_eq!(store.intern_atoms(&["a", "b", "a"]).unwrap(), [1, 2, 1]);
    assert_eq!(store.intern_atoms(&["b", "c"]).unwrap(), [2, 3]);
    assert_eq!(store.atom("d").unwrap(), None);
    // Closing writes the pending atoms.
    drop(store);
    assert_eq!(stored(&path), ["", "a", "b", "c"]);
    let store = ContentStore::open(&path).unwrap();
    assert_eq!(store.atom("c").unwrap(), Some(3));
    assert_eq!(store.atom_value(2).unwrap().as_deref(), Some("b"));
    assert_eq!(store.atom_value(9).unwrap(), None);
    assert_eq!(store.intern_atoms(&["d"]).unwrap(), [4]);
}

#[test]
fn two_handles_on_one_store_allocate_the_same_ids() {
    let path = path("shared");
    let one = ContentStore::open(&path).unwrap();
    let two = ContentStore::open(&path).unwrap();
    assert_eq!(one.intern_atoms(&["x"]).unwrap(), [1]);
    // One arena per database in a process: `two` sees `x` at once.
    assert_eq!(two.intern_atoms(&["y", "x"]).unwrap(), [2, 1]);
    assert_eq!(one.atom("y").unwrap(), Some(2));
    assert_eq!(one.intern_atoms(&["z"]).unwrap(), [3]);
    assert_eq!(two.atom_value(3).unwrap().as_deref(), Some("z"));
}

/// The invariant's write side: a store transaction appends the pending atoms before its own rows,
/// in the same commit.
#[test]
fn pending_atoms_commit_with_the_rows_that_name_them() {
    let path = path("with-rows");
    let store = ContentStore::open(&path).unwrap();
    let [p] = store.intern_atoms(&["p"]).unwrap()[..] else { panic!() };
    assert!(store.atoms.pending());
    assert_eq!(stored(&path), [""], "interning alone writes nothing");
    store
        .immediate_transaction(|transaction| {
            transaction.execute("INSERT INTO name_bindings(name_atom, digest, revision) VALUES (?1, zeroblob(32), 0)", [p])?;
            Ok(())
        })
        .unwrap();
    assert!(!store.atoms.pending());
    assert_eq!(stored(&path), ["", "p"]);
}

/// The invariant's guard: a commit that writes the store without appending the pending atoms is
/// refused, and one written as SQL commits through `atoms::commit`.
#[test]
fn a_commit_without_the_pending_atoms_is_refused() {
    let path = path("refused");
    let store = ContentStore::open(&path).unwrap();
    let [p] = store.intern_atoms(&["p"]).unwrap()[..] else { panic!() };
    let connection = store.open_connection().unwrap();
    let write = "BEGIN; INSERT INTO name_bindings(name_atom, digest, revision) VALUES (1, zeroblob(32), 0);";
    connection.execute_batch(write).unwrap();
    assert!(connection.execute_batch("COMMIT").is_err());
    assert!(connection.is_autocommit(), "the refused commit rolled back");
    let bound: i64 = connection.query_row("SELECT count(*) FROM name_bindings", [], |row| row.get(0)).unwrap();
    assert_eq!(bound, 0);
    // A transaction that writes only another database commits; the atoms stay pending.
    connection.execute_batch("ATTACH ':memory:' AS scratch; CREATE TABLE scratch.t(a); BEGIN; INSERT INTO scratch.t VALUES (1); COMMIT; DETACH scratch;").unwrap();
    assert!(store.atoms.pending());
    connection.execute_batch(write).unwrap();
    super::commit(&connection).unwrap();
    assert!(!store.atoms.pending());
    assert_eq!(stored(&path), ["", "p"]);
    assert_eq!(p, 1);
}

/// SQL reads atoms through the arena, pending ones included: by id, by value, by range and whole.
#[test]
fn sql_reads_atoms_through_the_arena() {
    let path = path("vtab");
    let store = ContentStore::open(&path).unwrap();
    store.intern_atoms(&["beta", "alpha", "alphabet", "gamma"]).unwrap();
    let connection = store.open_connection().unwrap();
    let one = |sql: &str| connection.query_row(sql, [], |row| row.get::<_, String>(0)).unwrap();
    assert_eq!(one("SELECT value FROM atoms WHERE atom = 2"), "alpha");
    assert_eq!(one("SELECT CAST(atom AS TEXT) FROM atoms WHERE value = 'gamma'"), "4");
    assert_eq!(one("SELECT group_concat(value, ',') FROM (SELECT value FROM atoms WHERE value >= 'alpha' AND value < 'alpi')"), "alpha,alphabet");
    assert_eq!(one("SELECT group_concat(value, ',') FROM (SELECT value FROM atoms WHERE value > 'alpha' AND value < x'')"), "alphabet,beta,gamma");
    assert_eq!(one("SELECT CAST(count(*) AS TEXT) FROM atoms"), "5");
    assert_eq!(one("SELECT CAST(count(*) AS TEXT) FROM atoms WHERE value = 'delta'"), "0");
    assert!(store.atoms.pending(), "reading wrote nothing");
}

/// A corpus scratch database borrows symbols, but has no atom_blocks table.
/// Contention makes setfix flush due atoms through this connection.
#[test]
fn shared_scratch_flushes_atoms_to_their_owning_store() {
    let owner = path("owner");
    let scratch_path = path("scratch");
    let store = ContentStore::open(&owner).unwrap();
    let mut scratch = rusqlite::Connection::open(&scratch_path).unwrap();
    store.share_atoms_with(&scratch_path);
    let arena = super::Atoms::install(&mut scratch).unwrap();
    assert!(std::sync::Arc::ptr_eq(&arena, &store.atoms));
    let ids = store.intern_atoms(&["borrowed"]).unwrap();
    arena.flush(&scratch).unwrap();
    assert!(!arena.pending());
    assert_eq!(stored(&owner), ["", "borrowed"]);
    let count: i64 = scratch.query_row("SELECT count(*) FROM sqlite_schema WHERE name = 'atom_blocks'", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 0, "scratch must not acquire an independent atom store");
    assert_eq!(ContentStore::open(&owner).unwrap().atom("borrowed").unwrap(), Some(ids[0]));
}

#[test]
fn shared_scratch_writes_do_not_use_the_owner_commit_guard() {
    let owner = path("owner-guard");
    let store = ContentStore::open(&owner).unwrap();
    let scratch_path = path("scratch-guard");
    let mut scratch = rusqlite::Connection::open(&scratch_path).unwrap();
    store.share_atoms_with(&scratch_path);
    let arena = super::Atoms::install(&mut scratch).unwrap();
    store.intern_atoms(&["pending"]).unwrap();
    scratch.execute_batch("CREATE TABLE work(x INTEGER); INSERT INTO work VALUES (1);").unwrap();
    assert!(arena.pending(), "scratch SQL does not commit the durable store");
    arena.flush(&scratch).unwrap();
    assert!(!arena.pending());
}

/// A store deleted and created again at its path, here by another process (a copy of a store made elsewhere), has
/// atoms of its own: this process, still holding the old store's, neither reads nor writes the new one with them.
#[test]
fn a_store_created_again_at_its_path_has_its_own_atoms() {
    let (at, elsewhere) = (path("recreated"), path("recreated-elsewhere"));
    let old = ContentStore::open(&at).unwrap();
    assert_eq!(old.intern_atoms(&["old"]).unwrap(), [1]);
    old.flush_atoms().unwrap();
    {
        let other = ContentStore::open(&elsewhere).unwrap();
        assert_eq!(other.intern_atoms(&["new", "old"]).unwrap(), [1, 2]);
    }
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(super::suffixed(&at, suffix));
    }
    copy_store(&elsewhere, &at);
    let store = ContentStore::open(&at).unwrap();
    assert!(!std::sync::Arc::ptr_eq(&store.atoms, &old.atoms));
    assert_eq!(store.atom_value(1).unwrap().as_deref(), Some("new"));
    assert_eq!(store.atom("old").unwrap(), Some(2));
    assert_eq!(store.intern_atoms(&["next"]).unwrap(), [3]);
    // Closing writes `next` to the new store, on a connection to it rather than the old one's: the path's writer
    // (which `old` still holds) writes it behind the close, so a reader waits for the queue (`written`) first.
    drop(store);
    old.written().unwrap();
    assert_eq!(stored(&at), ["", "new", "old", "next"]);
    drop(old);
}

const CHILD: &str = "RERAIL_ATOMS_CHILD";

/// Runs `test` in a child process of this test binary, with `CHILD` set to `role`.
fn child(test: &str, role: &str, path: &std::path::Path) -> std::process::Child {
    std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", test, "--nocapture", "--test-threads=1"])
        .env(CHILD, format!("{role}\n{}", path.display()))
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap()
}

fn role() -> Option<(String, std::path::PathBuf)> {
    let value = std::env::var(CHILD).ok()?;
    let (role, path) = value.split_once('\n')?;
    Some((role.to_owned(), path.into()))
}

/// The child of `a_crash_loses_only_pending_atoms_and_leaves_the_store_consistent`: commits a row
/// naming `kept`, interns `lost`, then dies without closing anything.
#[test]
#[ignore = "a child process of the crash test"]
fn crash_child() {
    let Some((role, path)) = role() else { return };
    assert_eq!(role, "crash");
    let store = ContentStore::open(&path).unwrap();
    let [kept] = store.intern_atoms(&["kept"]).unwrap()[..] else { panic!() };
    store
        .immediate_transaction(|transaction| {
            transaction.execute("INSERT INTO name_bindings(name_atom, digest, revision) VALUES (?1, zeroblob(32), 0)", [kept])?;
            Ok(())
        })
        .unwrap();
    store.intern_atoms(&["lost"]).unwrap();
    std::process::abort();
}

#[test]
fn a_crash_loses_only_pending_atoms_and_leaves_the_store_consistent() {
    let path = path("crash");
    drop(ContentStore::open(&path).unwrap());
    let status = child("content_store::atoms::tests::crash_child", "crash", &path).wait().unwrap();
    assert!(!status.success(), "the child aborted");
    let store = ContentStore::open(&path).unwrap();
    // The row the child committed names an atom that is stored; the pending one is gone.
    let named: i64 = store.connection.lock().unwrap().query_row("SELECT name_atom FROM name_bindings", [], |row| row.get(0)).unwrap();
    assert_eq!(store.atom_value(named as u32).unwrap().as_deref(), Some("kept"));
    assert_eq!(store.atom("lost").unwrap(), None);
    assert_eq!(stored(&path), ["", "kept"]);
    // The dead child's lease is free at once, and its lost id is given out again.
    assert_eq!(store.intern_atoms(&["new"]).unwrap(), [2]);
}

/// The child of `a_process_waiting_for_the_lease_gets_it_after_a_flush`: holds pending atoms and
/// keeps interning (as a chunked run does) until it sees the waiter, which makes it flush.
#[test]
#[ignore = "a child process of the lease test"]
fn lease_child() {
    let Some((role, path)) = role() else { return };
    assert_eq!(role, "lease");
    let store = ContentStore::open(&path).unwrap();
    store.intern_atoms(&["held"]).unwrap();
    std::fs::write(path.with_extension("ready"), "").unwrap();
    for i in 0..2_000 {
        std::thread::sleep(std::time::Duration::from_millis(5));
        store.intern_atoms(&[&format!("held{i}")]).unwrap();
        if !store.atoms.pending() {
            break;
        }
    }
    // Stays alive (holding nothing) until the parent is done.
    std::thread::sleep(std::time::Duration::from_secs(5));
}

#[test]
fn a_process_waiting_for_the_lease_gets_it_after_a_flush() {
    let path = path("lease");
    drop(ContentStore::open(&path).unwrap());
    let mut holder = child("content_store::atoms::tests::lease_child", "lease", &path);
    let ready = path.with_extension("ready");
    let started = std::time::Instant::now();
    while !ready.exists() {
        assert!(started.elapsed() < std::time::Duration::from_secs(30), "the child never started");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let store = ContentStore::open(&path).unwrap();
    // Waits for the child's lease; the child flushes and gives it back.
    let [mine] = store.intern_atoms(&["mine"]).unwrap()[..] else { panic!() };
    let _ = holder.kill();
    let _ = holder.wait();
    assert_eq!(store.atom_value(1).unwrap().as_deref(), Some("held"));
    assert!(mine > 1, "after the child's atoms");
    assert_eq!(store.atom_value(mine).unwrap().as_deref(), Some("mine"));
}

/// The store at `from` copied to `to` with its blocks (its `.atoms` folder, in place of any there): by SQLite
/// (`VACUUM INTO`), not as a file, since the store's checkpoint thread may still be writing the file's pages, and a
/// file copied then is torn ("database disk image is malformed"). A process still reading the folder it replaces keeps
/// its files (unlinked, not truncated).
fn copy_store(from: &std::path::Path, to: &std::path::Path) {
    rusqlite::Connection::open(from).unwrap().execute("VACUUM INTO ?1", [to.to_str().unwrap()]).unwrap();
    super::copy_blocks(from, to).unwrap();
}

/// The store at `path`, copied to a path of its own: opened there, it is a new process's (its atoms are mapped from
/// its files, not the ones this process holds for `path`).
fn elsewhere(path: &std::path::Path, label: &str) -> TestPath {
    let copy = self::path(label);
    copy_store(path, &copy);
    copy
}

/// A-R3 (decision 0401): a new process's atoms over a store of 100,000 hold none of them on its heap. It finds and
/// reads every one where it lies, and new ids continue after them.
#[test]
fn the_stores_atoms_are_mapped_not_read() {
    let path = path("mapped");
    let store = ContentStore::open(&path).unwrap();
    let words: Vec<String> = (0..100_000).map(|i| format!("word-{i}")).collect();
    let ids = store.intern_atoms(&words.iter().map(String::as_str).collect::<Vec<_>>()).unwrap();
    drop(store);
    let copy = elsewhere(&path, "mapped-again");
    let store = ContentStore::open(&copy).unwrap();
    let handle = store.atoms_handle();
    assert!(handle.ready_for_run().unwrap());
    for (w, &id) in words.iter().zip(&ids) {
        assert_eq!(handle.get(w), Some(id), "{w}");
        assert_eq!(handle.with_str(id, str::to_owned).as_deref(), Some(w.as_str()));
    }
    assert!(handle.bytes() < 1 << 20, "{} heap bytes over a mapped store", handle.bytes());
    assert!(handle.in_memory(super::Interner::mapped) > 1 << 20, "the blocks are mapped");
    handle.run_over();
    assert_eq!(store.intern_atoms(&["word-7", "new"]).unwrap(), [ids[7], 100_001]);
}

/// A-R4: atoms appended in a transaction that rolled back are not the store's. A new process reads none of them and
/// gives their ids out again; this process stores them with its next write.
#[test]
fn an_append_not_committed_is_not_the_stores() {
    let path = path("rolled-back");
    let store = ContentStore::open(&path).unwrap();
    assert_eq!(store.intern_atoms(&["a", "b"]).unwrap(), [1, 2]);
    let connection = store.open_connection().unwrap();
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert_eq!(store.atoms.write_pending(&connection).unwrap(), 3);
    connection.execute_batch("ROLLBACK").unwrap();
    assert!(store.atoms.pending(), "still pending here");
    let copy = elsewhere(&path, "rolled-back-again");
    let other = ContentStore::open(&copy).unwrap();
    assert_eq!(other.atom("a").unwrap(), None, "past the committed end");
    assert_eq!(other.intern_atoms(&["c"]).unwrap(), [1]);
    drop(other);
    assert_eq!(stored(&copy), ["", "c"], "appended over the rolled-back bytes");
    store.flush_atoms().unwrap();
    assert_eq!(stored(&path), ["", "a", "b"]);
}

/// A-R5: atoms another process committed are read here once the store's end is read again.
#[test]
fn another_process_commit_is_seen_by_catching_up() {
    let path = path("catching-up");
    let store = ContentStore::open(&path).unwrap();
    assert_eq!(store.intern_atoms(&["a"]).unwrap(), [1]);
    store.flush_atoms().unwrap();
    another(&path, &["p", "q"]);
    assert_eq!(store.atom("q").unwrap(), Some(3));
    assert_eq!(store.atom_value(2).unwrap().as_deref(), Some("p"));
    assert_eq!(store.intern_atoms(&["r", "p"]).unwrap(), [4, 2]);
}

/// The store's rows: each block's file name, first id and end.
fn blocks(path: &std::path::Path) -> Vec<(String, u32, u32)> {
    super::stored::rows(&rusqlite::Connection::open(path).unwrap()).unwrap().into_iter().map(|r| (r.name, r.first, r.upto)).collect()
}

/// A commit past a block's room starts the next, twice as large; past four deltas they are rolled into one base (A5).
/// Every atom reads back in a new process, and none is left on this one's heap.
#[test]
fn a_commit_past_a_blocks_room_starts_the_next() {
    let path = path("next-block");
    let store = ContentStore::open(&path).unwrap();
    let cap = super::stored::FIRST_CAP;
    let first: Vec<String> = (0..=cap).map(|i| format!("v{i}")).collect();
    store.intern_atoms(&first.iter().map(String::as_str).collect::<Vec<_>>()).unwrap();
    store.immediate_transaction(|_| Ok(())).unwrap();
    assert_eq!(blocks(&path), [("d1.blk".to_owned(), 1, 1 + cap), ("d2.blk".to_owned(), 1 + cap, 2 + cap)]);
    let words: Vec<String> = (0..70_000).map(|i| format!("w{i}")).collect();
    let ids = store.intern_atoms(&words.iter().map(String::as_str).collect::<Vec<_>>()).unwrap();
    store.immediate_transaction(|_| Ok(())).unwrap();
    assert!(!store.atoms.pending());
    assert!(store.atoms.state.read().atoms.pending.is_empty(), "the pending atoms' blocks, wholly stored, let go (decision 0478 step 2a)");
    let rows = blocks(&path);
    assert!(rows.len() == 1 && rows[0].0.starts_with('b') && (rows[0].1, rows[0].2) == (1, 70_002 + cap), "one base: {rows:?}");
    for i in [0, 69_999] {
        assert_eq!(store.atom(&words[i]).unwrap(), Some(ids[i]), "read through the base here");
    }
    // Decision 0478 step 2b: every atom read through the directory, by the reads written once in CLIF, is the
    // store's: across the base, and the pending blocks of atoms interned since.
    let more: Vec<String> = (0..300).map(|i| format!("x{i}")).collect();
    let more_ids = store.intern_atoms(&more.iter().map(String::as_str).collect::<Vec<_>>()).unwrap();
    {
        let state = store.atoms.state.read();
        let dir = state.atoms.dir();
        let last = *more_ids.last().unwrap();
        for id in (0..=last).step_by(97).chain([1, cap, cap + 1, 70_001 + cap, more_ids[0], last]) {
            // SAFETY: a host directory over blocks this process maps.
            let got = unsafe { rerail_blocks::atoms::str_in(dir.root(), id) };
            assert_eq!(got, state.atoms.str(id).map(str::as_bytes), "id {id}");
        }
        // SAFETY: as above.
        assert_eq!(unsafe { rerail_blocks::atoms::str_in(dir.root(), last + 1) }, None, "past every atom");
    }
    drop(store);
    let copy = elsewhere(&path, "next-block-again");
    let store = ContentStore::open(&copy).unwrap();
    for i in [0, 15, 16, 65_536, 69_999] {
        assert_eq!(store.atom(&words[i]).unwrap(), Some(ids[i]));
        assert_eq!(store.atom_value(ids[i]).unwrap().as_deref(), Some(words[i].as_str()));
    }
    assert_eq!(store.atom_value(cap + 1).unwrap().as_deref(), Some(first[cap as usize].as_str()));
}

/// A-R7: deltas rolled into a base read the same ids, here and in a new process; a view of the old files mapped before
/// still reads them after they are removed; the next write removes them.
#[test]
fn a_roll_up_keeps_every_id() {
    let path = path("roll-up");
    let store = ContentStore::open(&path).unwrap();
    let mut words: Vec<String> = Vec::new();
    let mut ids: Vec<u32> = Vec::new();
    let mut batch = |n: usize, store: &ContentStore| {
        let more: Vec<String> = (words.len()..words.len() + n).map(|i| format!("r{i}")).collect();
        ids.extend(store.intern_atoms(&more.iter().map(String::as_str).collect::<Vec<_>>()).unwrap());
        store.flush_atoms().unwrap();
        words.extend(more);
    };
    batch(40, &store);
    assert!(blocks(&path).iter().all(|(name, _, _)| name.starts_with('d')), "deltas yet");
    // A reader of the deltas as they are now.
    let connection = rusqlite::Connection::open(&path).unwrap();
    let rows = super::stored::rows(&connection).unwrap();
    let mut old = super::stored::Stored::new(Some(&path));
    old.learn(&rows);
    old.view_to(super::stored::end_of(&rows)).unwrap();
    for _ in 0..8 {
        batch(60, &store);
    }
    let rows = blocks(&path);
    assert!(rows[0].0.starts_with('b') && rows.len() <= 5, "rolled up: {rows:?}");
    // The next write after a roll-up's commit removes what it replaced (a write that is a roll-up itself leaves them).
    for _ in 0..3 {
        batch(1, &store);
        if blocks(&path).last().is_some_and(|(name, _, _)| name.starts_with('d')) {
            break;
        }
    }
    let named: std::collections::BTreeSet<String> = blocks(&path).into_iter().map(|(name, _, _)| name).collect();
    let files: std::collections::BTreeSet<String> = std::fs::read_dir(super::stored::atoms_dir(&path)).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(files, named, "only the blocks the rows name are left");
    for (w, &id) in words.iter().zip(&ids).take(40) {
        assert_eq!(old.str(id), Some(w.as_str()), "the old files still read");
    }
    for (w, &id) in words.iter().zip(&ids) {
        assert_eq!(store.atom(w).unwrap(), Some(id));
    }
    drop(store);
    let copy = elsewhere(&path, "roll-up-again");
    let store = ContentStore::open(&copy).unwrap();
    for (w, &id) in words.iter().zip(&ids) {
        assert_eq!(store.atom(w).unwrap(), Some(id), "{w} in a new process");
    }
}

/// A-R6 (0401 A7, by the store's floor): a store of an older floor (its atoms in segment rows) is refused, not
/// misread; the store is a cache, and this build's has another file name.
#[test]
fn an_older_floor_store_is_refused() {
    let path = path("older-floor");
    drop(ContentStore::open(&path).unwrap());
    rusqlite::Connection::open(&path).unwrap().pragma_update(None, "user_version", 5).unwrap();
    let refused = ContentStore::open(&path).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(refused.contains("older than this build's floor"), "{refused}");
}

/// Decision 0111's measurement, by hand: `RERAIL_ATOMS_PROBE=<store>` times a process's first lookup, an id's
/// string and a new atom on that store, reading the atoms alone (no store set-up).
#[test]
#[ignore = "a measurement on a store named by RERAIL_ATOMS_PROBE"]
fn probe_a_store() {
    let Some(path) = std::env::var_os("RERAIL_ATOMS_PROBE").map(std::path::PathBuf::from) else { return };
    let atoms = super::Atoms::for_path(&path);
    let t = std::time::Instant::now();
    let found = atoms.lookup("node").unwrap();
    eprintln!("first lookup: {found:?} in {:.3} s; heap {:.1} MB", t.elapsed().as_secs_f64(), atoms.bytes() as f64 / 1e6);
    let t = std::time::Instant::now();
    let value = atoms.value(4_000_000).unwrap();
    eprintln!("id 4000000: {:?} in {:.3} s; heap {:.1} MB", value.map(|v| v.chars().take(40).collect::<String>()), t.elapsed().as_secs_f64(), atoms.bytes() as f64 / 1e6);
    let t = std::time::Instant::now();
    let fresh = format!("probe-{}", std::process::id());
    let id = atoms.intern(&fresh).unwrap();
    eprintln!("a new atom: {id} of {} in {:.3} s; heap {:.1} MB, mapped {:.1} MB", atoms.len(), t.elapsed().as_secs_f64(), atoms.bytes() as f64 / 1e6, atoms.in_memory(super::Interner::mapped) as f64 / 1e6);
}

/// Decision 0114 §4: atoms a component makes ahead of the run's order are reserved, not counted by the commit
/// check, until it retires; then they are pending as any, and the next commit writing rows appends them.
#[test]
fn reserved_atoms_wait_for_their_component_then_commit_with_rows() {
    let path = path("reserved");
    let store = ContentStore::open(&path).unwrap();
    let arena = std::sync::Arc::clone(&store.atoms);
    // Made on another thread inside a reservation: held there, not pending to a commit here.
    let reserved = std::thread::scope(|s| {
        s.spawn(|| {
            let r = super::reserving(&arena);
            assert_eq!(arena.intern_all(&["r1", "r2"]).unwrap(), [1, 2]);
            r.end()
        })
        .join()
        .unwrap()
    });
    assert_eq!(reserved.len(), 2);
    assert!(!store.atoms.pending(), "reserved atoms are no row's yet");
    // A commit that writes rows meanwhile is not refused.
    let connection = store.open_connection().unwrap();
    connection.execute_batch("BEGIN; INSERT INTO name_bindings(name_atom, digest, revision) VALUES (0, zeroblob(32), 0); COMMIT;").unwrap();
    // Retired: pending, so a commit that does not append them is refused again.
    drop(reserved);
    assert!(store.atoms.pending());
    connection.execute_batch("BEGIN; INSERT INTO name_bindings(name_atom, digest, revision) VALUES (1, zeroblob(32), 1);").unwrap();
    assert!(connection.execute_batch("COMMIT").is_err());
    connection.execute_batch("BEGIN; INSERT INTO name_bindings(name_atom, digest, revision) VALUES (1, zeroblob(32), 1);").unwrap();
    super::commit(&connection).unwrap();
    assert!(!store.atoms.pending());
    // Another arena's atoms are not this reservation's.
    let other_path = self::path("reserved-other");
    let other = ContentStore::open(&other_path).unwrap();
    let r = super::reserving(&arena);
    other.intern_atoms(&["o"]).unwrap();
    assert!(other.atoms.pending(), "made in another arena: pending as usual");
    assert_eq!(r.end().len(), 0);
}

/// A run after the first (a phase's) holds every stored atom as the first did: the lease given back at the first's end
/// and taken again keeps its lookups out of the database (decision 0104). Before, every miss read the store's key.
#[test]
fn a_second_run_still_holds_every_atom() {
    let path = path("second-run");
    let store = ContentStore::open(&path).unwrap();
    let atoms = store.atoms_handle();
    assert!(atoms.ready_for_run().unwrap());
    store.intern_atoms(&["a", "b"]).unwrap();
    atoms.run_over();
    // The first run's end: its atoms written, the lease given back.
    store.flush_atoms().unwrap();
    assert!(!atoms.leased.load(Ordering::Acquire));
    // A new string before the next run's `ready_for_run` takes the lease again: still complete.
    assert_eq!(store.intern_atoms(&["c"]).unwrap(), [3]);
    assert!(atoms.complete.load(Ordering::Acquire));
    assert!(atoms.ready_for_run().unwrap());
    assert_eq!(store.intern_atoms(&["a", "d"]).unwrap(), [1, 4]);
}

/// A reservation made while another is open on the thread (a component's job run by another's wait) holds its own
/// atoms; the outer one goes on when it ends, holding its own before and after.
#[test]
fn a_reservation_inside_another_holds_its_own_atoms() {
    let path = path("nested");
    let store = ContentStore::open(&path).unwrap();
    let arena = std::sync::Arc::clone(&store.atoms);
    let outer = super::reserving(&arena);
    arena.intern_all(&["outer-1"]).unwrap();
    let inner = super::reserving(&arena);
    arena.intern_all(&["inner-1", "inner-2"]).unwrap();
    let inner = inner.end();
    arena.intern_all(&["outer-2"]).unwrap();
    let outer = outer.end();
    assert_eq!((inner.len(), outer.len()), (2, 2));
    assert!(!store.atoms.pending(), "both still reserved");
    drop(inner);
    assert!(store.atoms.pending(), "the inner one retired: its atoms are pending");
    drop(outer);
}

/// Decision 0170: ids given out here while another process stored atoms past this one's end are not the store's. This
/// process forks: its pending strings are stored under ids of their own (a string the store has takes its id), and a
/// kept entry's atom columns are the store's ids in its block.
#[test]
fn atoms_another_process_stored_past_this_ones_end_are_mapped_after_a_fork() {
    let path = path("forked");
    let store = ContentStore::open(&path).unwrap();
    store.intern_atoms(&["a"]).unwrap();
    store.flush_atoms().unwrap();
    assert_eq!(store.intern_atoms(&["b", "y"]).unwrap(), [2, 3]);
    // Another process stores its own atoms 2 and 3 (as one taking no lease would): `z`, and this one's `y`.
    another(&path, &["z", "y"]);
    store.flush_atoms().unwrap();
    let handle = store.atoms_handle();
    assert!(handle.forked());
    assert!(!handle.pending(), "every atom has its store id");
    assert_eq!(stored(&path), ["", "a", "z", "y", "b"], "`b` stored after the other's atoms, `y` found in them");
    // A kept entry of `b`, `y` and `a` holds the store's ids in its block (4, 3, 1), mapped as it is written.
    let space = [7u8; 32];
    let put = crate::content_store::kept_facts::KeptPut {
        rel: "r".into(),
        ident: [1; 32],
        part: 0,
        digest: 9,
        reads: Vec::new(),
        types: vec![b's', b'i'],
        what: crate::content_store::kept_facts::KeptWhat::Flat { widths: rerail_blocks::vector![8, 8], rows: rerail_blocks::vector![2, 2, 3, 3, 1, 1], n: 3, levels: None, given: false },
    };
    super::mapped(|| store.queue_kept(space, vec![put]));
    let row = store.kept_row(&space).unwrap().expect("written");
    let kept = store.open_kept(&space, &row).unwrap();
    let view = kept.view();
    let entry = rerail_blocks::kept::KeptRead::get(&view, &rerail_blocks::kept::FactKey { rel: "r", ident: &[1; 32], part: 0 }).expect("the entry");
    // In canonical order (decision 0456 K3): by the store's ids.
    let rows: Vec<Vec<i64>> = (0..3).map(|id| entry.row(id)).collect();
    assert_eq!(rows, [vec![1, 1], vec![3, 3], vec![4, 2]], "the store's ids (b 4, y 3, a 1), each integer with its row");
    // The other process's `z` is held here under an id of this process's.
    let mut z = [2i64];
    handle.to_local(StoreIds::as_stored(&mut z)).unwrap();
    assert_eq!(handle.with_str(z[0] as u32, str::to_owned).as_deref(), Some("z"));
    // A write that does not declare its symbols mapped is refused after the fork.
    store.intern_atoms(&["c"]).unwrap();
    let refused = store.immediate_transaction(|_| Ok(()));
    assert!(refused.as_ref().is_err_and(|e| e.to_string().contains(super::DIVERGED_TEXT)), "{refused:?}");
}

/// Decision 0170: a write transaction that finds another process's atoms past this one's end, with none pending here,
/// moves this process's end past them before its rows are made. An id given out while it runs is not one of theirs,
/// and the process does not fork. (The writer's batch gave ids under the other's, and its later job forked: the
/// jobs before had written rows of them, unmapped.)
#[test]
fn a_write_that_finds_the_store_moved_gives_ids_past_its_end() {
    let path = path("moved");
    let store = ContentStore::open(&path).unwrap();
    store.intern_atoms(&["a"]).unwrap();
    store.flush_atoms().unwrap();
    another(&path, &["z"]);
    let given = store.in_one_transaction(|| store.intern_atoms(&["b"]).unwrap()).unwrap();
    assert_eq!(given, [3], "past the other process's `z`");
    let handle = store.atoms_handle();
    assert!(!handle.forked());
    assert!(!handle.pending());
    assert_eq!(stored(&path), ["", "a", "z", "b"]);
}

/// After a fork (decision 0170), the derivation tables name the store's ids: a forked process binds and records under
/// them, reads its own bindings back, and another process reading the store finds them under the right strings. The
/// `atoms` table answers in the store's ids.
#[test]
fn a_forked_processs_derivations_name_the_stores_atoms() {
    let path = path("forked-derivations");
    let store = ContentStore::open(&path).unwrap();
    store.intern_atoms(&["a"]).unwrap();
    store.flush_atoms().unwrap();
    assert_eq!(store.intern_atoms(&["b", "y"]).unwrap(), [2, 3]);
    another(&path, &["z", "y"]);
    store.flush_atoms().unwrap();
    assert!(store.atoms_handle().forked());
    let digest = |n: u8| [n; 32];
    let record = crate::content_store::DerivationRecord {
        output: "b".into(),
        producer: digest(1),
        inputs: vec![crate::content_store::DerivationInput { name: Some("y".into()), digest: digest(2) }],
        digest: digest(3),
        status: 0,
        cost: 0,
    };
    store.record_derivations(&[record]).unwrap();
    store.bind_names(&[crate::content_store::Binding { name: "y".into(), digest: digest(2) }]).unwrap();
    // The store holds `b` as 4 and `y` as 3 (the other process's).
    let connection = rusqlite::Connection::open(&path).unwrap();
    let bound: Vec<(i64, Vec<u8>)> = connection.prepare("SELECT name_atom, digest FROM name_bindings ORDER BY name_atom").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
    assert_eq!(bound, [(3, digest(2).to_vec()), (4, digest(3).to_vec())]);
    let input: i64 = connection.query_row("SELECT name_atom FROM derivation_inputs WHERE output_atom = 4", [], |r| r.get(0)).unwrap();
    assert_eq!(input, 3);
    // This process reads them back by its own ids.
    assert_eq!(store.binding("b").unwrap(), Some(digest(3)));
    assert!(store.derivation_valid("b").unwrap());
    assert_eq!(store.derivations_reading(&["y".into()]).unwrap().iter().map(|d| d.output.as_str()).collect::<Vec<_>>(), ["b"]);
    assert_eq!(store.readers_of(3).unwrap(), [2], "`y` is read by `b`, in this process's ids");
    assert_eq!(store.derivation_of(2).unwrap().unwrap().inputs, [(Some(3), digest(2))]);
    // Another process reads the store's.
    crate::content_store::forget_held_snapshot(path.to_str().unwrap());
    let other = ContentStore::open(&path).unwrap();
    assert_eq!(other.binding("b").unwrap(), Some(digest(3)));
    assert_eq!(other.binding("z").unwrap(), None);
    // The `atoms` table answers in the store's ids, as the tables it joins hold them.
    let sql = store.open_connection().unwrap();
    let value = |atom: i64| sql.query_row("SELECT value FROM atoms WHERE atom = ?1", [atom], |r| r.get::<_, String>(0)).unwrap();
    assert_eq!([value(1), value(3), value(4)], ["a", "y", "b"]);
    let atom: i64 = sql.query_row("SELECT atom FROM atoms WHERE value = 'b'", [], |r| r.get(0)).unwrap();
    assert_eq!(atom, 4);
}

/// An append a fork made in a transaction that rolled back is forgotten: its atoms are pending again, and the next
/// write stores them.
#[test]
fn a_forks_append_rolled_back_is_forgotten() {
    let path = path("fork-rollback");
    let store = ContentStore::open(&path).unwrap();
    store.intern_atoms(&["a"]).unwrap();
    store.flush_atoms().unwrap();
    store.intern_atoms(&["b"]).unwrap();
    another(&path, &["z"]);
    let handle = store.atoms_handle();
    let connection = store.open_connection().unwrap();
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    handle.begin_write(&connection).unwrap();
    assert!(handle.forked());
    let mut b = [2i64];
    handle.to_store(&connection, ProcessIds::of_process(&mut b)).unwrap();
    assert_eq!(b, [3]);
    connection.execute_batch("ROLLBACK").unwrap();
    assert_eq!(stored(&path), ["", "a", "z"]);
    store.flush_atoms().unwrap();
    assert!(!handle.pending());
    assert_eq!(stored(&path), ["", "a", "z", "b"]);
}

/// Decision 0170's guard: every integer, untyped and blob column of a store's schema is classified, as holding atoms
/// (`ATOM_COLUMNS`, with how it is mapped across numberings) or not (`NOT_ATOMS`). A new column in neither fails here,
/// so a column of atoms is never kept unmapped; an entry for a column the schema no longer has fails too.
#[test]
fn every_stored_column_is_classified() {
    let path = path("classified");
    let store = ContentStore::open(&path).unwrap();
    drop(store);
    let connection = rusqlite::Connection::open(&path).unwrap();
    let columns: Vec<(String, String, String)> = connection
        .prepare("SELECT m.name, p.name, upper(p.type) FROM sqlite_master m, pragma_table_info(m.name) p WHERE m.type = 'table' ORDER BY 1, p.cid")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let atoms: std::collections::HashSet<(&str, &str)> = super::map::ATOM_COLUMNS.iter().map(|(t, c, _)| (*t, *c)).collect();
    let not: std::collections::HashSet<(&str, &str)> = super::map::NOT_ATOMS.iter().flat_map(|(t, cs)| cs.iter().map(move |c| (*t, *c))).collect();
    let both: Vec<_> = atoms.intersection(&not).collect();
    assert!(both.is_empty(), "classified both ways: {both:?}");
    let unclassified: Vec<String> = columns
        .iter()
        .filter(|(_, _, ty)| ty.starts_with("INT") || ty.starts_with("BLOB") || ty == "ANY" || ty.is_empty())
        .filter(|(t, c, _)| !atoms.contains(&(t.as_str(), c.as_str())) && !not.contains(&(t.as_str(), c.as_str())))
        .map(|(t, c, ty)| format!("{t}.{c} ({ty})"))
        .collect();
    assert!(unclassified.is_empty(), "columns neither in ATOM_COLUMNS nor NOT_ATOMS (decision 0170): {unclassified:?}");
    let tables: std::collections::HashSet<&str> = columns.iter().map(|(t, _, _)| t.as_str()).collect();
    let present: std::collections::HashSet<(&str, &str)> = columns.iter().map(|(t, c, _)| (t.as_str(), c.as_str())).collect();
    let stale: Vec<_> = atoms.iter().chain(&not).filter(|(t, c)| tables.contains(t) && !present.contains(&(*t, *c))).collect();
    assert!(stale.is_empty(), "classified columns the schema does not have: {stale:?}");
}

/// A forked process (decision 0170) holds strings under ids of its own: one the store has and one it lacks. Both are
/// pending until a write's rows name them, and that write maps them (`to_store`). So nothing queues a write for them:
/// not the run's end, not the store's close.
#[test]
fn a_forked_processs_unnamed_atoms_queue_no_write() {
    let path = path("forked-unnamed");
    let store = ContentStore::open(&path).unwrap();
    assert_eq!(store.intern_atoms(&["a"]).unwrap(), [1]);
    store.flush_atoms().unwrap();
    assert_eq!(store.intern_atoms(&["c"]).unwrap(), [2]);
    another(&path, &["b", "c"]);
    let handle = store.atoms_handle();
    // A stored row read names another process's atom past this one's end: this process forks.
    handle.hold_named(&[3]).unwrap();
    assert!(handle.forked());
    store.intern_atoms(&["d"]).unwrap();
    assert!(handle.pending(), "neither has a store id");
    assert!(!handle.flush_wanted(), "and no write needs one");
    drop(store);
    drop(handle);
    assert_eq!(stored(&path), ["", "a", "b", "c"], "nothing stored at the close");
}

/// Decision 0478 step 2a: the run's pending atoms are in atoms blocks, the stored chain's format, each string where
/// a block holds it (a device reads it there): none in a heap buffer. The chain grows past its first block, every
/// atom is found by its string and read by its id, and the blocks of atoms stored since are let go.
#[test]
fn pending_atoms_live_in_atoms_blocks() {
    use super::{Interner, PENDING_FIRST_CAP, hash_str};
    let mut i = Interner::default();
    let n = 3 * PENDING_FIRST_CAP as usize;
    let strings: Vec<String> = (0..n).map(|k| format!("atom-{k}-{}", "x".repeat(k % 37))).collect();
    let ids: Vec<Sym> = strings.iter().map(|s| i.intern(s)).collect();
    assert_eq!(ids, (1..=n as Sym).collect::<Vec<_>>(), "dense ids from the first");
    let blocks = i.pending_regions();
    assert!(blocks.len() >= 2, "the chain grew past its first block: {blocks:?}");
    for (s, &id) in strings.iter().zip(&ids) {
        let got = i.str(id).expect("a pending atom's string");
        assert_eq!(got, s);
        let at = got.as_ptr() as usize;
        assert!(blocks.iter().any(|&(lo, hi)| at >= lo && at + got.len() <= hi), "{s}: read where an atoms block holds it");
        assert_eq!(i.get(s), Some(id));
        assert_eq!(i.hash_of(id), Some(hash_str(s)));
    }
    assert_eq!(i.intern(&strings[5]), ids[5], "interned again: the same id");
    assert_eq!(i.pending(), (n, strings.iter().map(String::len).sum()));
    assert_eq!(i.len(), n + 1);
}
