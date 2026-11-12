use super::super::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn path(label: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("rerail-atoms-{label}-{}-{nonce}.sqlite", std::process::id()))
}

/// The database's segments, read on a connection of its own: (first, count).
fn segments(path: &std::path::Path) -> Vec<(i64, i64)> {
    let connection = rusqlite::Connection::open(path).unwrap();
    let mut statement = connection.prepare("SELECT first, length(ends) / 4 FROM atom_segment ORDER BY first").unwrap();
    statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?))).unwrap().collect::<Result<_, _>>().unwrap()
}

#[test]
fn atoms_are_dense_stable_and_survive_reopening() {
    let path = path("dense");
    let store = ContentStore::open(&path).unwrap();
    assert_eq!(store.atom("").unwrap(), Some(0));
    assert_eq!(store.intern_atoms(&["a", "b", "a"]).unwrap(), [1, 2, 1]);
    assert_eq!(store.intern_atoms(&["b", "c"]).unwrap(), [2, 3]);
    assert_eq!(store.atom("d").unwrap(), None);
    // Closing writes the pending atoms: one segment for the three.
    drop(store);
    assert_eq!(segments(&path), [(0, 1), (1, 3)]);
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
    assert_eq!(segments(&path), [(0, 1)], "interning alone writes nothing");
    store
        .immediate_transaction(|transaction| {
            transaction.execute("INSERT INTO name_bindings(name_atom, digest, revision) VALUES (?1, zeroblob(32), 0)", [p])?;
            Ok(())
        })
        .unwrap();
    assert!(!store.atoms.pending());
    assert_eq!(segments(&path), [(0, 1), (1, 1)]);
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
    assert_eq!(segments(&path), [(0, 1), (1, 1)]);
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

/// A corpus scratch database borrows symbols, but has no atom_segment table.
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
    assert_eq!(segments(&owner), [(0, 1), (1, 1)]);
    let count: i64 = scratch.query_row("SELECT count(*) FROM sqlite_schema WHERE name = 'atom_segment'", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 0, "scratch must not acquire an independent atom store");
    assert_eq!(ContentStore::open(&owner).unwrap().atom("borrowed").unwrap(), Some(ids[0]));
}

#[test]
fn shared_scratch_writes_do_not_use_the_owner_commit_guard() {
    let store = ContentStore::open(&path("owner-guard")).unwrap();
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
    rusqlite::Connection::open(&elsewhere).unwrap().execute("VACUUM INTO ?1", [at.to_str().unwrap()]).unwrap();
    let store = ContentStore::open(&at).unwrap();
    assert!(!std::sync::Arc::ptr_eq(&store.atoms, &old.atoms));
    assert_eq!(store.atom_value(1).unwrap().as_deref(), Some("new"));
    assert_eq!(store.atom("old").unwrap(), Some(2));
    assert_eq!(store.intern_atoms(&["next"]).unwrap(), [3]);
    // Closing queues `next` for the new store's writer, on its connection rather than the old one's; a handle
    // opened again shares that writer and waits for it.
    drop(store);
    ContentStore::open(&at).unwrap().written().unwrap();
    assert_eq!(segments(&at), [(0, 1), (1, 2), (3, 1)]);
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
    assert_eq!(segments(&path), [(0, 1), (1, 1)]);
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

/// The database at `path`, copied to a path of its own: opened there, it is a new process's (its atoms are read
/// from disk, not the ones this process holds for `path`).
fn elsewhere(path: &std::path::Path, label: &str) -> std::path::PathBuf {
    rusqlite::Connection::open(path).unwrap().execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    let copy = self::path(label);
    std::fs::copy(path, &copy).unwrap();
    copy
}

fn key_rows(path: &std::path::Path) -> (i64, Option<i64>) {
    let connection = rusqlite::Connection::open(path).unwrap();
    let rows = connection.query_row("SELECT count(*) FROM atom_key", [], |row| row.get(0)).unwrap();
    let upto = connection.query_row("SELECT upto FROM atom_key_end", [], |row| row.get(0)).ok();
    (rows, upto)
}

/// Decision 0111: a reopened store finds an atom by its key and reads only that atom's segment; new ids continue
/// after the stored ones.
#[test]
fn a_reopened_store_reads_only_the_segments_it_looks_up() {
    let path = path("keyed");
    let store = ContentStore::open(&path).unwrap();
    let first: Vec<String> = (0..1000).map(|i| format!("first-{i}")).collect();
    store.intern_atoms(&first.iter().map(String::as_str).collect::<Vec<_>>()).unwrap();
    store.flush_atoms().unwrap();
    let second: Vec<String> = (0..1000).map(|i| format!("second-{i}")).collect();
    let ids = store.intern_atoms(&second.iter().map(String::as_str).collect::<Vec<_>>()).unwrap();
    drop(store);
    assert_eq!(segments(&path), [(0, 1), (1, 1000), (1001, 1000)]);
    assert_eq!(key_rows(&path), (2001, Some(2001)), "a key per atom, written with its segment");
    let store = ContentStore::open(&elsewhere(&path, "keyed-again")).unwrap();
    assert_eq!(store.atom("second-7").unwrap(), Some(ids[7]));
    let handle = store.atoms_handle();
    assert!(handle.state.read().atoms.holds(ids[999]), "the atom's segment is held");
    assert!(!handle.state.read().atoms.holds(1), "a segment no lookup reached is not read");
    assert_eq!(store.atom_value(3).unwrap().as_deref(), Some("first-2"));
    assert_eq!(store.intern_atoms(&["first-5", "third"]).unwrap(), [6, 2001]);
}

/// A key row whose hash is another string's (a collision, forced here): the lookup compares the strings.
#[test]
fn a_hash_collision_finds_the_right_atom() {
    let path = path("collision");
    let store = ContentStore::open(&path).unwrap();
    assert_eq!(store.intern_atoms(&["x", "y"]).unwrap(), [1, 2]);
    drop(store);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute("INSERT INTO atom_key(h, id) VALUES (?1, 1)", [super::hash_str("y") as i64]).unwrap();
    drop(connection);
    let store = ContentStore::open(&elsewhere(&path, "collision-again")).unwrap();
    assert_eq!(store.atom("y").unwrap(), Some(2));
    assert_eq!(store.atom("x").unwrap(), Some(1));
    assert_eq!(store.intern_atoms(&["y", "z"]).unwrap(), [2, 3]);
}

/// A store from before the key (its segments alone): keyed once, when an atom is first looked up.
#[test]
fn an_older_store_is_keyed_with_its_next_write() {
    let path = path("older");
    let store = ContentStore::open(&path).unwrap();
    assert_eq!(store.intern_atoms(&["a", "b", "c"]).unwrap(), [1, 2, 3]);
    drop(store);
    rusqlite::Connection::open(&path).unwrap().execute_batch("DROP TABLE atom_key; DROP TABLE atom_key_end;").unwrap();
    let path = elsewhere(&path, "older-again");
    let store = ContentStore::open(&path).unwrap();
    // A lookup reads the segments, and writes nothing (decision 0104): the store has no key yet.
    assert_eq!(store.atom("b").unwrap(), Some(2));
    assert_eq!(store.intern_atoms(&["d"]).unwrap(), [4]);
    // The next write keys every segment before it, then appends its own.
    store.flush_atoms().unwrap();
    assert_eq!(key_rows(&path), (5, Some(5)));
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
    eprintln!("first lookup: {found:?} in {:.3} s; held {:.1} MB", t.elapsed().as_secs_f64(), atoms.bytes() as f64 / 1e6);
    let t = std::time::Instant::now();
    let value = atoms.value(4_000_000).unwrap();
    eprintln!("id 4000000: {:?} in {:.3} s; held {:.1} MB", value.map(|v| v.chars().take(40).collect::<String>()), t.elapsed().as_secs_f64(), atoms.bytes() as f64 / 1e6);
    let t = std::time::Instant::now();
    let fresh = format!("probe-{}", std::process::id());
    let id = atoms.intern(&fresh).unwrap();
    eprintln!("a new atom: {id} of {} in {:.3} s; held {:.1} MB", atoms.len(), t.elapsed().as_secs_f64(), atoms.bytes() as f64 / 1e6);
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
    let other = ContentStore::open(&self::path("reserved-other")).unwrap();
    let r = super::reserving(&arena);
    other.intern_atoms(&["o"]).unwrap();
    assert!(other.atoms.pending(), "made in another arena: pending as usual");
    assert_eq!(r.end().len(), 0);
}
