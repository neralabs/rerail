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
    // Closing writes `next` to the new store, on a connection to it rather than the old one's: the path's writer
    // (which `old` still holds) writes it behind the close, so a reader waits for the queue (`written`) first.
    drop(store);
    old.written().unwrap();
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

/// Decision 0170: ids given out here while another process stored atoms past this one's end are not the store's. This
/// process forks: its pending strings are stored under ids of their own (a string the store has takes its id), and a
/// frame's symbol columns are the store's ids in the store and this process's in memory.
#[test]
fn atoms_another_process_stored_past_this_ones_end_are_mapped_after_a_fork() {
    let path = path("forked");
    let store = ContentStore::open(&path).unwrap();
    store.intern_atoms(&["a"]).unwrap();
    store.flush_atoms().unwrap();
    assert_eq!(store.intern_atoms(&["b", "y"]).unwrap(), [2, 3]);
    // Another process stores its own atoms 2 and 3 (as one taking no lease would): `z`, and this one's `y`.
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute("INSERT INTO atom_segment(first, ends, bytes) VALUES (2, x'0100000002000000', CAST('zy' AS BLOB))", [])
        .unwrap();
    store.flush_atoms().unwrap();
    let handle = store.atoms_handle();
    assert!(handle.forked());
    assert!(!handle.pending(), "every atom has its store id");
    assert_eq!(segments(&path), [(0, 1), (1, 1), (2, 2), (4, 1)], "`b` stored after the other's atoms, `y` found in them");
    // A frame of `b`, `y` and `a` holds the store's ids there (4, 3, 1), and this process's when read back.
    let space = [7u8; 32];
    let rel = crate::content_store::FrameRel { name: "r".into(), ident: [1; 32] };
    let rows = crate::content_store::FrameRows { types: vec![b's', b'i'], rows: vec![2, 2, 3, 3, 1, 1], n: 3 };
    let put = crate::content_store::FramePut { rel: rel.clone(), part: 0, digest: 9, reads: Vec::new(), rows };
    super::mapped(|| store.keep_frames(&space, &[put], &[], None)).unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    let data: Vec<u8> = connection.query_row("SELECT data FROM frames WHERE rel = 'r'", [], |r| r.get(0)).unwrap();
    let mut cols = vec![Vec::new(); 2];
    crate::content_store::cross_rows::decode(2, 3, &data, &mut cols).unwrap();
    assert_eq!(cols[0], [4, 3, 1], "the store's ids");
    assert_eq!(cols[1], [2, 3, 1], "an integer column is as it was");
    crate::content_store::forget_frames_snapshot(path.to_str().unwrap());
    let reopened = ContentStore::open(&path).unwrap();
    let (_, frame) = reopened.admit_all_parts(&space, &rel).unwrap().pop().unwrap();
    assert_eq!(frame.rows().unwrap().rows, [2, 2, 3, 3, 1, 1], "this process's ids");
    // The other process's `z` is held here under an id of this process's.
    let mut z = [2i64];
    handle.to_local(&mut z).unwrap();
    assert_eq!(handle.with_str(z[0] as u32, str::to_owned).as_deref(), Some("z"));
    // A write that does not declare its symbols mapped is refused after the fork.
    store.intern_atoms(&["c"]).unwrap();
    let refused = store.immediate_transaction(|_| Ok(()));
    assert!(refused.as_ref().is_err_and(|e| e.to_string().contains(super::DIVERGED_TEXT)), "{refused:?}");
}

/// After a fork (decision 0170), the derivation tables name the store's ids: a forked process binds and records under
/// them, reads its own bindings back, and another process reading the store finds them under the right strings. The
/// `atoms` table, whose ids are this process's, refuses.
#[test]
fn a_forked_processs_derivations_name_the_stores_atoms() {
    let path = path("forked-derivations");
    let store = ContentStore::open(&path).unwrap();
    store.intern_atoms(&["a"]).unwrap();
    store.flush_atoms().unwrap();
    assert_eq!(store.intern_atoms(&["b", "y"]).unwrap(), [2, 3]);
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute("INSERT INTO atom_segment(first, ends, bytes) VALUES (2, x'0100000002000000', CAST('zy' AS BLOB))", [])
        .unwrap();
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
    crate::content_store::forget_frames_snapshot(path.to_str().unwrap());
    let other = ContentStore::open(&path).unwrap();
    assert_eq!(other.binding("b").unwrap(), Some(digest(3)));
    assert_eq!(other.binding("z").unwrap(), None);
    let refused = store.open_connection().unwrap().query_row("SELECT value FROM atoms WHERE atom = 1", [], |r| r.get::<_, String>(0));
    assert!(refused.as_ref().is_err_and(|e| e.to_string().contains(super::DIVERGED_TEXT)), "{refused:?}");
}

/// A segment a fork appended in a transaction that rolled back is forgotten: its atoms are pending again, and the
/// next write stores them.
#[test]
fn a_forks_segment_rolled_back_is_forgotten() {
    let path = path("fork-rollback");
    let store = ContentStore::open(&path).unwrap();
    store.intern_atoms(&["a"]).unwrap();
    store.flush_atoms().unwrap();
    store.intern_atoms(&["b"]).unwrap();
    rusqlite::Connection::open(&path).unwrap().execute("INSERT INTO atom_segment(first, ends, bytes) VALUES (2, x'01000000', CAST('z' AS BLOB))", []).unwrap();
    let handle = store.atoms_handle();
    let connection = store.open_connection().unwrap();
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    handle.begin_write(&connection).unwrap();
    assert!(handle.forked());
    let mut b = [2i64];
    handle.to_store(&connection, &mut b).unwrap();
    assert_eq!(b, [3]);
    connection.execute_batch("ROLLBACK").unwrap();
    assert_eq!(segments(&path), [(0, 1), (1, 1), (2, 1)]);
    store.flush_atoms().unwrap();
    assert!(!handle.pending());
    assert_eq!(segments(&path), [(0, 1), (1, 1), (2, 1), (3, 1)]);
}

/// Pending atoms are written as segments of at most `SEGMENT_BYTES` each (one atom at least), so a run's strings past
/// SQLite's largest blob still store: split here at 10 bytes, and read back whole.
#[test]
fn pending_atoms_are_written_in_segments_of_bounded_size() {
    let mut interner = super::Interner::default();
    for s in ["aaaa", "bbbb", "cccc", "dddddddddddddd", "e"] {
        interner.intern(s);
    }
    let (cut, keys) = interner.segments(1, 10);
    let shape: Vec<(usize, usize, Vec<u8>)> = cut.iter().map(|(first, ends, bytes)| (*first, ends.len() / 4, bytes.clone())).collect();
    assert_eq!(shape, [(1, 2, b"aaaabbbb".to_vec()), (3, 1, b"cccc".to_vec()), (4, 1, b"dddddddddddddd".to_vec()), (5, 1, b"e".to_vec())]);
    assert_eq!(keys.len(), 5);
    let path = path("segments");
    let store = ContentStore::open(&path).unwrap();
    let words: Vec<String> = (0..300).map(|i| format!("{i:0>10000}")).collect();
    let ids = store.intern_atoms(&words.iter().map(String::as_str).collect::<Vec<_>>()).unwrap();
    // A transaction that waits for the store (`flush_atoms` gives way to another writer).
    store.immediate_transaction(|_| Ok(())).unwrap();
    assert!(segments_of(&path) > 3, "3 MB of strings in segments of 1 MB: {:?}", segments(&path));
    drop(store);
    let store = ContentStore::open(&elsewhere(&path, "segments-again")).unwrap();
    assert_eq!(store.atom_value(ids[299]).unwrap().as_deref(), Some(words[299].as_str()));
    assert_eq!(store.atom(&words[170]).unwrap(), Some(ids[170]));
}

fn segments_of(path: &std::path::Path) -> usize {
    segments(path).len()
}
