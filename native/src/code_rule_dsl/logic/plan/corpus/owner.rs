//! Which process a store belongs to: one at a time, as the product's machine-wide store belongs to its main daemon
//! alone (`main-instance.lock`, `src/engine/native/main-instance.ts`), and every other process keeps one of its own.
//!
//! Processes that share a store share its atoms lease (decision 0049), which a run takes before its first wave and
//! keeps across its waves. A second process's run waits behind it for up to `LEASE_WAIT` and then fails `content store
//! writer is busy`; decision 0160 measured the lease deadlocking between processes, and keeps sharing it a design
//! question (0160, 0170). So the engine never shares one: a process owns a store by an exclusive OS lock on
//! `<store>-owner.lock`, held for the process's life and released by the OS however it ends, so nothing is reclaimed
//! by guessing. `<store>-owner.json` beside it names the holder, for a message only.
//!
//! - A store the engine picks (`slot`: the project's, testing mode's) is the first of `<name>.sqlite`,
//!   `<name>-2.sqlite`, `<name>-3.sqlite`, … that no other process owns. It never waits: a run alone, or runs one
//!   after another, keep using the first and what it caches; runs at once each get their own.
//! - A store the user names (`RERAIL_TAPE_CACHE`) is waited for (`named`): a process that finds another owning it
//!   says so and starts once that one ends, or gives up after `NAMED_WAIT` with an error that names it.
//! - A store whose lock cannot be made or taken at all (no file locks: the browser build, the only process on its
//!   filesystem) is used as before, without an owner; said once, unless the platform has no file locks.
//!
//! Each answer is the process's for its life: asked again, a store is owned, used without a lock, or refused as it
//! was the first time.

use std::collections::HashMap;
use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// How long a process waits for another to end with a store the user named for both.
pub const NAMED_WAIT: Duration = Duration::from_secs(600);

/// What this process holds of a store.
enum Claim {
    /// The store is this process's: its lock file, locked, kept open to the process's end.
    Owned(#[allow(dead_code)] File),
    /// Its lock could not be made or taken (`unlocked`): used without an owner.
    Unlocked,
    /// Another process kept it past the wait: why this process runs without it.
    Refused(String),
}

/// The stores this process claimed, by path. Held only to look up and record, never across a wait.
static CLAIMS: LazyLock<Mutex<HashMap<PathBuf, Claim>>> = LazyLock::new(Default::default);
/// The slot this process owns of each base store it asked for (`slot`).
static SLOTS: LazyLock<Mutex<HashMap<PathBuf, PathBuf>>> = LazyLock::new(Default::default);
/// Held by the thread waiting for a named store (`named_within`): another thread of the process asking for one waits
/// for its answer, and never takes the lock file a second time against its own process.
static WAITING: Mutex<()> = Mutex::new(());

fn claims() -> MutexGuard<'static, HashMap<PathBuf, Claim>> {
    CLAIMS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// `store` with `suffix` appended to its file name.
fn beside(store: &Path, suffix: &str) -> PathBuf {
    let mut name = store.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// The store's lock file, opened (made, with its folder, where missing).
fn lock_file(store: &Path) -> std::io::Result<File> {
    if let Some(dir) = store.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    open_lock(store, true)
}

/// `store`'s lock file, opened (made where missing and `make`). On Windows it is opened without sharing its deletion,
/// so no cleanup removes it while a process has it open (`remove_lock`); std's default shares it.
fn open_lock(store: &Path, make: bool) -> std::io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(make).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_SHARE_READ | FILE_SHARE_WRITE: not FILE_SHARE_DELETE.
        options.share_mode(0x1 | 0x2);
    }
    let path = beside(store, "-owner.lock");
    // A cleanup's removal has the file open for a moment (`remove_lock`): a sharing violation (32), or access denied
    // (5) while its deletion is pending. Asked again a moment later, the file is there, or gone and made anew.
    #[cfg(windows)]
    for _ in 0..20 {
        match options.open(&path) {
            Err(error) if matches!(error.raw_os_error(), Some(5 | 32)) => std::thread::sleep(Duration::from_millis(5)),
            opened => return opened,
        }
    }
    options.open(&path)
}

/// Whether `file` (locked) is still the lock file at `store`'s path: a cleanup unlinks a lock file last, holding it
/// (`remove_lock`), so a process that opened it before and locks it after has a lock no one else can see.
fn still_there(file: &File, store: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (file.metadata(), std::fs::metadata(beside(store, "-owner.lock"))) {
            (Ok(held), Ok(named)) => (held.dev(), held.ino()) == (named.dev(), named.ino()),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        // Elsewhere a lock file is not removed while a process has it open (`open_lock`): the file locked is the path's.
        let _ = (file, store);
        true
    }
}

/// Names this process as `store`'s holder, replacing the record whole so a reader never sees half of one.
fn record(store: &Path) {
    let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let text = serde_json::json!({ "pid": std::process::id(), "since": since }).to_string();
    let (tmp, path) = (beside(store, &format!("-owner.json.{}", std::process::id())), beside(store, "-owner.json"));
    if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// The process that holds `store`, as its record names it.
pub fn holder(store: &Path) -> String {
    let record: Option<serde_json::Value> = std::fs::read(beside(store, "-owner.json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let (Some(pid), since) = (record.as_ref().and_then(|r| r["pid"].as_u64()), record.as_ref().and_then(|r| r["since"].as_u64())) else {
        return "another process (it has not named itself yet)".to_owned();
    };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    match since {
        Some(since) => format!("process {pid} (holding it for {} s)", now.saturating_sub(since)),
        None => format!("process {pid}"),
    }
}

/// `store` used without an owner, its lock file failing with `error`: said, unless the platform has no file locks.
fn unlocked(store: &Path, error: &std::io::Error) -> Claim {
    if error.kind() != std::io::ErrorKind::Unsupported {
        eprintln!("rerail-engine: cannot lock {} ({error}); this process uses the store without owning it", beside(store, "-owner.lock").display());
    }
    Claim::Unlocked
}

/// What this process already answered for `store`: `Some(true)` owned or used without a lock, `Some(false)` refused.
fn answered(store: &Path, claims: &HashMap<PathBuf, Claim>) -> Option<bool> {
    claims.get(store).map(|c| !matches!(c, Claim::Refused(_)))
}

/// Takes `store` for this process without waiting: whether this process may use it now (owned, or without a lock).
fn try_own(store: &Path, claims: &mut HashMap<PathBuf, Claim>) -> bool {
    if let Some(usable) = answered(store, claims) {
        return usable;
    }
    loop {
        let file = match lock_file(store) {
            Ok(file) => file,
            Err(error) => {
                claims.insert(store.to_path_buf(), unlocked(store, &error));
                return true;
            }
        };
        match file.try_lock() {
            // A lock file a cleanup unlinked meanwhile: the one at the path now is the store's.
            Ok(()) if !still_there(&file, store) => continue,
            Ok(()) => {
                record(store);
                claims.insert(store.to_path_buf(), Claim::Owned(file));
                return true;
            }
            Err(TryLockError::WouldBlock) => return false,
            Err(TryLockError::Error(error)) => {
                claims.insert(store.to_path_buf(), unlocked(store, &error));
                return true;
            }
        }
    }
}

/// The `k`th slot of `base` (`<name>-<k>.sqlite`); the first is `base`.
fn numbered(base: &Path, k: usize) -> PathBuf {
    if k == 1 {
        return base.to_path_buf();
    }
    let stem = base.file_stem().unwrap_or_default().to_string_lossy();
    let name = match base.extension() {
        Some(ext) => format!("{stem}-{k}.{}", ext.to_string_lossy()),
        None => format!("{stem}-{k}"),
    };
    base.with_file_name(name)
}

/// The first slot of `base` that no other process owns, this process's from here on: the same one at every call.
pub fn slot(base: &Path) -> PathBuf {
    let mut slots = SLOTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(path) = slots.get(base) {
        return path.clone();
    }
    let mut claims = claims();
    let path = (1..).map(|k| numbered(base, k)).find(|path| try_own(path, &mut claims)).expect("a slot no process owns");
    slots.insert(base.to_path_buf(), path.clone());
    path
}

/// Claims `store`, which the user named, for this process: at once when no other process owns it, else once that
/// one ends, waiting at most `NAMED_WAIT`. Asked again, answers as it did the first time. The error names the holder,
/// and is said once as it is made: every caller but the run's opener (`tape_cache`) then sees no store.
pub fn named(store: &Path) -> Result<(), String> {
    named_within(store, NAMED_WAIT)
}

pub(super) fn named_within(store: &Path, wait: Duration) -> Result<(), String> {
    let refused = |claims: &HashMap<PathBuf, Claim>| match claims.get(store) {
        Some(Claim::Refused(why)) => Err(why.clone()),
        _ => Ok(()),
    };
    {
        let claims = claims();
        if claims.contains_key(store) {
            return refused(&claims);
        }
    }
    let _one = WAITING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // Another thread of the process may have answered while this one waited for it.
    {
        let claims = claims();
        if claims.contains_key(store) {
            return refused(&claims);
        }
    }
    let claim = wait_for(store, wait);
    let mut claims = claims();
    claims.insert(store.to_path_buf(), claim);
    refused(&claims)
}

/// `store`'s claim once its lock is this process's, or another process kept it past `wait`; no map held meanwhile.
fn wait_for(store: &Path, wait: Duration) -> Claim {
    let mut said = false;
    let deadline = Instant::now() + wait;
    let mut pause = Duration::from_millis(5);
    loop {
        let file = match lock_file(store) {
            Ok(file) => file,
            Err(error) => return unlocked(store, &error),
        };
        loop {
            match file.try_lock() {
                Ok(()) if !still_there(&file, store) => break,
                Ok(()) => {
                    record(store);
                    return Claim::Owned(file);
                }
                Err(TryLockError::Error(error)) => return unlocked(store, &error),
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    if !said {
                        eprintln!(
                            "rerail-engine: the store {} belongs to {}; waiting for it to end (at most {} s): a store is one process's at a time",
                            store.display(),
                            holder(store),
                            wait.as_secs()
                        );
                        said = true;
                    }
                    std::thread::sleep(pause.min(deadline.saturating_duration_since(Instant::now())));
                    pause = (pause * 2).min(Duration::from_millis(500));
                }
                Err(TryLockError::WouldBlock) => {
                    let why = format!(
                        "the store {} belongs to {}, which kept it past the {} s this process waited: a store is one process's at a time (decisions 0049, 0160); name another with RERAIL_TAPE_CACHE, or leave it unset for one of the engine's own",
                        store.display(),
                        holder(store),
                        wait.as_secs()
                    );
                    eprintln!("rerail-engine: {why}; this process runs without it");
                    return Claim::Refused(why);
                }
            }
        }
    }
}

/// Why this process runs without `store`, which the user named: another process kept it past the wait (`named`).
pub fn refusal(store: &Path) -> Option<String> {
    match claims().get(store) {
        Some(Claim::Refused(why)) => Some(why.clone()),
        _ => None,
    }
}

/// Removes the files `names` in `dir` (a cleanup's: another build's, unused for a day), each store's whole
/// (`remove_store`): a store a live process owns keeps its files.
pub fn remove_unowned(dir: &Path, names: &[String]) {
    let mut stores: HashMap<&str, Vec<&str>> = HashMap::new();
    for name in names {
        match name.find(".sqlite") {
            Some(at) => stores.entry(&name[..at + ".sqlite".len()]).or_default().push(name),
            None => {
                let _ = std::fs::remove_file(dir.join(name));
            }
        }
    }
    for (store, files) in stores {
        remove_store(dir, &dir.join(store), &files);
    }
}

/// Removes `files` of `store` while holding its owner lock, the lock file last (`remove_lock`); nothing when a
/// process, this one included, owns it.
fn remove_store(dir: &Path, store: &Path, files: &[&str]) {
    let Some(held) = unowned(store) else { return };
    let lock = beside(store, "-owner.lock");
    let lock_name = lock.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    for name in files.iter().filter(|n| **n != lock_name) {
        let _ = std::fs::remove_file(dir.join(name));
    }
    if files.contains(&lock_name.as_str()) {
        remove_lock(&lock, held);
    }
}

/// `store`'s owner lock taken for a cleanup: `Some(None)` where it has no lock file, `None` where a process owns it
/// or its lock file cannot be opened.
fn unowned(store: &Path) -> Option<Option<File>> {
    if claims().contains_key(store) {
        return None;
    }
    match open_lock(store, false) {
        Ok(file) => file.try_lock().ok().map(|()| Some(file)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(None),
        // Not known to be free: left as it is.
        Err(_) => None,
    }
}

/// Removes the lock file at `lock`, held (`held`) by the cleanup. On unix it is unlinked while held: a process that
/// opened it before and locks it after finds it gone (`still_there`) and takes the new one. Elsewhere it is let go
/// first, then removed: a process that opened it meanwhile keeps it there (`open_lock` shares no deletion), and the
/// removal fails.
fn remove_lock(lock: &Path, held: Option<File>) {
    let Some(held) = held else { return };
    #[cfg(unix)]
    {
        let _ = std::fs::remove_file(lock);
        drop(held);
    }
    #[cfg(not(unix))]
    {
        drop(held);
        let _ = std::fs::remove_file(lock);
    }
}

#[cfg(test)]
mod tests {
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

    /// A cleanup removes an unused store's files, its lock file with them, and leaves every file of a store another
    /// process owns (here an open file of the test's own, locked, as the OS sees another process's).
    #[test]
    fn a_cleanup_leaves_a_store_another_process_owns() {
        let dir = tempfile::tempdir().unwrap();
        let names = |store: &str| -> Vec<String> { ["", "-wal", "-owner.lock", "-owner.json", ".files"].iter().map(|s| format!("{store}{s}")).collect() };
        let mut all: Vec<String> = names("held.sqlite").into_iter().chain(names("free.sqlite")).chain(["loose".to_owned()]).collect();
        for name in &all {
            std::fs::write(dir.path().join(name), "").unwrap();
        }
        let other = std::fs::OpenOptions::new().read(true).write(true).open(dir.path().join("held.sqlite-owner.lock")).unwrap();
        other.lock().unwrap();
        super::remove_unowned(dir.path(), &all);
        let mut left: Vec<String> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        all = names("held.sqlite");
        all.sort();
        assert_eq!(left, all, "the held store's files stay; the free store's and the loose file go");
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
        std::fs::write(dir.join(format!("{name}.ready")), path.to_string_lossy().as_bytes()).unwrap();
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
}
