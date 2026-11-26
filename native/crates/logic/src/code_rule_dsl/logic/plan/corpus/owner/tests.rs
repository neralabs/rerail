    use std::path::{Path, PathBuf};
    use std::process::{Child, Command};
    use std::time::{Duration, Instant};

    const CHILD: &str = "code_rule_dsl::logic::plan::corpus::owner::tests::a_run_holding_its_store";

    /// A process of this test binary running `a_run_holding_its_store` as `name`, in `dir`, over `base` (a slot of it)
    /// or the store `RERAIL_TAPE_CACHE` names.
    fn start(dir: &Path, name: &str, base: Option<&Path>, named: Option<&Path>) -> Child {
        let mut child = Command::new(std::env::current_exe().unwrap());
        child.args(["--exact", CHILD, "--ignored", "--test-threads", "1", "--nocapture"]).env("RERAIL_OWNER_CHILD_DIR", dir).env("RERAIL_OWNER_CHILD_NAME", name);
        child.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        match (base, named) {
            (Some(base), _) => child.env("RERAIL_OWNER_CHILD_BASE", base).env_remove("RERAIL_TAPE_CACHE"),
            (None, Some(named)) => child.env("RERAIL_TAPE_CACHE", named),
            (None, None) => unreachable!(),
        };
        super::super::switches::started(|| child.spawn()).unwrap()
    }

    /// The store `name` holds once it is in its run (its `.ready` file), or the child's output where it ended first.
    fn ready(dir: &Path, name: &str, child: &mut Child) -> PathBuf {
        let file = dir.join(format!("{name}.ready"));
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if let Ok(text) = std::fs::read_to_string(&file) {
                return PathBuf::from(text);
            }
            if child.try_wait().unwrap().is_some() || Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                let (mut stdout, mut stderr) = (String::new(), String::new());
                if let Some(pipe) = child.stdout.as_mut() {
                    let _ = std::io::Read::read_to_string(pipe, &mut stdout);
                }
                if let Some(pipe) = child.stderr.as_mut() {
                    let _ = std::io::Read::read_to_string(pipe, &mut stderr);
                }
                panic!("{name} ended before its run: {stdout}\n{stderr}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn finished(name: &str, child: Child) -> String {
        let out = child.wait_with_output().unwrap();
        let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success() && stdout.contains("1 passed"), "{name}: {stdout}\n{stderr}");
        stderr.into_owned()
    }

    /// Runs started at once from one place took one store (bug 5 of the Django rules): the second waited for the
    /// first's atoms lease, which a run keeps across its waves, for `LEASE_WAIT`, and failed `content store writer is
    /// busy`. Each now owns a slot of the store: the second takes the next while the first holds its own, and a run
    /// after both takes the first again, with what it cached.
    #[test]
    fn runs_at_once_own_a_store_each_and_a_later_run_takes_the_first_again() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("project.sqlite");
        let mut a = start(dir.path(), "a", Some(&base), None);
        let a_store = ready(dir.path(), "a", &mut a);
        let mut b = start(dir.path(), "b", Some(&base), None);
        let b_store = ready(dir.path(), "b", &mut b);
        assert_eq!(a_store, base, "the first run owns the first slot");
        assert_eq!(b_store, dir.path().join("project-2.sqlite"), "the second, run at once, the next");
        std::fs::write(dir.path().join("release"), "").unwrap();
        finished("a", a);
        finished("b", b);
        let mut c = start(dir.path(), "c", Some(&base), None);
        assert_eq!(ready(dir.path(), "c", &mut c), base, "a run after both owns the first again");
        finished("c", c);
    }

    /// A store the user names for two processes at once is the first's until it ends: the second says whose it is,
    /// waits, and then runs on it (`a.done` there: the first's last act). Before, both opened it, and the second's run
    /// waited for the first's atoms lease and went on once the first's run gave it back, before the first process
    /// ended (or failed `busy` past `LEASE_WAIT`).
    #[test]
    fn a_named_store_is_its_owners_until_it_ends() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("named.sqlite");
        let mut a = start(dir.path(), "a", None, Some(&store));
        assert_eq!(ready(dir.path(), "a", &mut a), store);
        let mut b = start(dir.path(), "b", None, Some(&store));
        // b is started and waits: it says for whom.
        let deadline = Instant::now() + Duration::from_secs(120);
        while !dir.path().join("b.started").exists() {
            assert!(Instant::now() < deadline && b.try_wait().unwrap().is_none(), "b never started");
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_millis(300));
        assert!(!dir.path().join("b.ready").exists(), "b ran on a's store while a held it");
        std::fs::write(dir.path().join("release"), "").unwrap();
        assert_eq!(ready(dir.path(), "b", &mut b), store);
        finished("a", a);
        let said = finished("b", b);
        assert!(said.contains(&format!("belongs to process {}", std::fs::read_to_string(dir.path().join("a.pid")).unwrap())), "b says whose the store is: {said}");
    }

    /// Past the wait, a process gives up the named store with an error that names its holder.
    #[test]
    fn past_the_wait_the_named_store_is_refused_naming_its_holder() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("held.sqlite");
        let held = super::lock_file(&store).unwrap();
        held.lock().unwrap();
        std::fs::write(super::beside(&store, "-owner.json"), r#"{"pid":4242,"since":0}"#).unwrap();
        let why = super::named_within(&store, Duration::from_millis(50)).unwrap_err();
        assert!(why.contains("belongs to process 4242") && why.contains("RERAIL_TAPE_CACHE"), "{why}");
        assert_eq!(super::refusal(&store).as_deref(), Some(why.as_str()), "kept: the process runs without it");
        assert_eq!(super::named(&store), Err(why), "asked again, the same answer");
    }

    /// A named store in a folder not made yet is this process's from its first ask: the lock file is made with the
    /// folder, so no later ask, with the store open, finds it to take.
    #[test]
    fn a_named_store_in_a_new_folder_is_owned_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("new/folder/named.sqlite");
        assert_eq!(super::named_within(&store, Duration::from_millis(50)), Ok(()));
        let other = std::fs::OpenOptions::new().read(true).write(true).open(super::beside(&store, "-owner.lock")).unwrap();
        assert!(matches!(other.try_lock(), Err(std::fs::TryLockError::WouldBlock)), "the lock is this process's");
        assert_eq!(super::named_within(&store, Duration::from_millis(50)), Ok(()), "asked again, the same answer");
    }

    /// A cleanup removes an unused store's files and folders, its lock file with them, and leaves every file of a store another
    /// process owns (here an open file of the test's own, locked, as the OS sees another process's).
    #[test]
    fn a_cleanup_leaves_a_store_another_process_owns() {
        let dir = tempfile::tempdir().unwrap();
        let names = |store: &str| -> Vec<String> { ["", "-wal", "-owner.lock", "-owner.json", ".files"].iter().map(|s| format!("{store}{s}")).collect() };
        let mut all: Vec<String> = names("held.sqlite").into_iter().chain(names("free.sqlite")).chain(["loose".to_owned()]).collect();
        for name in &all {
            std::fs::write(dir.path().join(name), "").unwrap();
        }
        // A store's folders (its objects, its kept facts), each with a file in it: they go with the store's files.
        let folders = |store: &str| -> Vec<String> { [".objects", ".facts"].iter().map(|s| format!("{store}{s}")).collect() };
        for name in folders("held.sqlite").into_iter().chain(folders("free.sqlite")) {
            std::fs::create_dir(dir.path().join(&name)).unwrap();
            std::fs::write(dir.path().join(&name).join("x"), "").unwrap();
            all.push(name);
        }
        let other = std::fs::OpenOptions::new().read(true).write(true).open(dir.path().join("held.sqlite-owner.lock")).unwrap();
        other.lock().unwrap();
        super::remove_unowned(dir.path(), &all);
        let mut left: Vec<String> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        all = names("held.sqlite").into_iter().chain(folders("held.sqlite")).collect();
        all.sort();
        assert_eq!(left, all, "the held store's files and folders stay; the free store's and the loose file go");
    }

    /// One of the processes above: owns its store (a slot of `RERAIL_OWNER_CHILD_BASE`, or the one `RERAIL_TAPE_CACHE`
    /// names, through `tape_cache` as a run opens it), takes the atoms lease as a run does before its first wave, gives
    /// out an atom, and keeps them until the parent releases it.
    #[test]
    #[ignore = "run by the tests above"]
    fn a_run_holding_its_store() {
        let Some(dir) = std::env::var_os("RERAIL_OWNER_CHILD_DIR").map(PathBuf::from) else { return };
        let name = std::env::var("RERAIL_OWNER_CHILD_NAME").unwrap();
        std::fs::write(dir.join(format!("{name}.pid")), std::process::id().to_string()).unwrap();
        std::fs::write(dir.join(format!("{name}.started")), "").unwrap();
        let (path, store) = match std::env::var_os("RERAIL_OWNER_CHILD_BASE") {
            Some(base) => {
                let path = super::slot(Path::new(&base));
                let store = crate::content_store::ContentStore::open(&path).unwrap();
                (path, store)
            }
            None => {
                let store = super::super::tape_cache().unwrap().expect("the named store");
                // The store is this process's only once the other's has ended.
                let other = dir.join(if name == "a" { "b.started" } else { "a.started" });
                assert!(!other.exists() || dir.join(if name == "a" { "b.done" } else { "a.done" }).exists(), "{name} runs on a store another process holds");
                (super::super::tape_cache_path().expect("the named store"), store)
            }
        };
        let atoms = store.atoms_handle();
        atoms.ready_for_run().unwrap();
        store.intern_atoms(&[&format!("{name}'s own")]).unwrap();
        // Written beside it and renamed: the parent reads the file whole, never one created and not yet written.
        let part = dir.join(format!("{name}.ready.part"));
        std::fs::write(&part, path.to_string_lossy().as_bytes()).unwrap();
        std::fs::rename(&part, dir.join(format!("{name}.ready"))).unwrap();
        let deadline = Instant::now() + Duration::from_secs(120);
        while !dir.join("release").exists() {
            assert!(Instant::now() < deadline, "never released");
            std::thread::sleep(Duration::from_millis(10));
        }
        atoms.run_over();
        store.flush_atoms().unwrap();
        // The run is over and its lease given back; the process ends a moment later, and only then is its store free.
        std::thread::sleep(Duration::from_millis(500));
        std::fs::write(dir.join(format!("{name}.done")), "").unwrap();
    }
