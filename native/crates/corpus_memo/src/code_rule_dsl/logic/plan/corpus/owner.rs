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
pub fn beside(store: &Path, suffix: &str) -> PathBuf {
    let mut name = store.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// The store's lock file, opened (made, with its folder, where missing).
pub fn lock_file(store: &Path) -> std::io::Result<File> {
    if let Some(dir) = store.parent().filter(|d| !d.as_os_str().is_empty()) {
        crate::content_store::prepare_cache_directory(dir)?;
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

pub fn named_within(store: &Path, wait: Duration) -> Result<(), String> {
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

/// Removes the files and folders `names` in `dir` (a cleanup's: another build's, unused for a day), each store's whole
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
        // A store's folders (its objects, kept facts, atoms, memo) hold most of its bytes: they go whole.
        let path = dir.join(name);
        let _ = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
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

// cratify: `tests` is rerail-logic's (it mounts it: it reads the language, `rr::parse_version`, or the corpus's root)
