//! Each test's own store, copied from a template (the data-layer repository's SQLite fixtures, `conform/conftest.py`).
//!
//! A test that does not name a store (`TEST_TAPE_CACHE`, `RERAIL_TAPE_CACHE`) gets one of its own on first use: a
//! copy of the template, cloned (APFS `clonefile`, else a copy) into a directory of its own, and removed when the
//! test's thread ends. No test sees what another left behind, and none waits on another's leases (O146).
//!
//! The template holds what every test would otherwise make first: the rule library's phase 0 (each module's tape and
//! its loads, `bootstrap.rr`'s plan) and the core program's plan. It is made once per build (keyed by the build's
//! identities, `build_ids`), by the first test process that wants it, under a file lock; the processes after it find
//! it ready (`<key>.ready`). It is written to a staging store, drained (atoms, queued writes), and published with
//! `VACUUM INTO` and a rename, so a reader never sees half of one. A binary that registers no language (a lower
//! crate's) cannot run phase 0: its template is an empty store, and it says so once.
//!
//! A copy is a new store: the template has no identity, and a copy takes its own (`store_identity`) at its first open,
//! as a store created does. So a copy is a clone and nothing more.
use std::path::{Path, PathBuf};

/// Where templates are kept, one per build.
fn templates() -> PathBuf {
    std::env::temp_dir().join("rerail-store-templates")
}

/// Where the tests' stores are: one directory each, `<pid>-<n>`, so that sweeping them reads only theirs.
fn copies() -> PathBuf {
    std::env::temp_dir().join("rerail-test-stores")
}

/// Another build's template unused this long is removed: every build of the engine has its own (an edit is a new
/// build), so they are many while one works.
const STALE: std::time::Duration = std::time::Duration::from_secs(3600);

/// The template for this build: made once, by whichever process gets there first.
pub fn template() -> &'static Path {
    static PATH: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    PATH.get_or_init(|| {
        let key = format!("{}-{}", &crate::code_rule_dsl::build_ids::engine()[..16], &crate::code_rule_dsl::build_ids::grammar()[..16]);
        let dir = templates();
        std::fs::create_dir_all(&dir).expect("the templates' directory");
        let path = dir.join(format!("{key}.sqlite"));
        let ready = dir.join(format!("{key}.ready"));
        let _lock = Lock::take(&dir.join(format!("{key}.lock")));
        if !(path.exists() && std::fs::read_to_string(&ready).is_ok_and(|k| k == key)) {
            // Test plumbing, not a run's SQL: the strict mode (`RERAIL_SQL_STRICT`) leaves it alone.
            crate::code_rule_dsl::logic::profile::behind(|| make(&path));
            std::fs::write(&ready, &key).expect("the template's ready mark");
        } else {
            // Used: its mark is fresh, so no other process takes it for stale.
            let _ = std::fs::write(&ready, &key);
        }
        sweep_templates(&dir, &key);
        path
    })
}

/// Makes the template at `path`: a staging store filled on a thread of its own (it names its store, as a test does),
/// drained, and published whole.
fn make(path: &Path) {
    let staging = path.with_extension(format!("{}.{}.build", std::process::id(), nanos()));
    std::fs::create_dir_all(&staging).expect("the template's staging directory");
    let store = staging.join("store.sqlite");
    let filling = store.clone();
    let filled = std::thread::Builder::new()
        .name("store template".into())
        // Test plumbing, not a run's SQL: the strict mode (`RERAIL_SQL_STRICT`) leaves it alone.
        .spawn(move || crate::code_rule_dsl::logic::profile::behind(|| fill(&filling)))
        .expect("the template's thread")
        .join()
        .unwrap_or_else(|panic| Err(panic.downcast_ref::<String>().cloned().or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()));
    if let Err(why) = filled {
        eprintln!("rerail: the test stores' template holds no library ({why}); every test's store starts empty");
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging).expect("the template's staging directory");
        drop(crate::content_store::ContentStore::open(&store).expect("an empty store"));
    }
    let published = path.with_extension(format!("{}.{}.tmp", std::process::id(), nanos()));
    let db = rusqlite::Connection::open(&store).expect("the template's staging store");
    db.execute("VACUUM INTO ?1", [published.to_string_lossy()]).expect("the template, written whole");
    drop(db);
    // Published as a copy needs it: in WAL mode (a store's first open would switch it), and with no identity (each
    // copy takes its own at its first open, `ContentStore::initialize`), so that a copy is a clone and nothing more.
    let db = rusqlite::Connection::open(&published).expect("the published template");
    db.execute_batch("DELETE FROM store_identity; PRAGMA journal_mode=WAL;").expect("the template, made ready to copy");
    drop(db);
    std::fs::rename(&published, path).expect("the template, published");
    let _ = std::fs::remove_dir_all(&staging);
}

/// Phase 0 over every library module and the core program, then a run of the core program over an empty corpus:
/// what a test's first run would do before it reads its own files. Then collected, as a run's collection would leave
/// it: a process whose include graphs were already in memory (the template is made by a test, mid-suite) writes
/// `core.rr`'s tape with no manifest naming it, and a copy's first collection, due ten minutes after the template's
/// decision, took it out from under the test (`a_tests_store_is_a_copy_of_the_template_and_goes_with_its_test`).
fn fill(store: &Path) -> std::result::Result<(), String> {
    super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(store.to_owned()));
    let lib = std::fs::canonicalize(crate::code_rule_dsl::logic::rr::library()).map_err(|e| e.to_string())?;
    let mut seeds = vec![super::phase0::Seed::Text { t_shape: false, includes: vec![(0, "core.rr".to_owned())] }];
    let mut modules = Vec::new();
    rule_files(&lib, &mut modules);
    modules.sort();
    seeds.extend(modules.into_iter().map(super::phase0::Seed::File));
    super::phase0::graphs(&seeds).map_err(|e| e.to_string())?;
    let program = "#include \"core.rr\"\n.decl defs(s: symbol)\ndefs(x) :- kind(n, \"function_definition\"), field(n, \"name\", i), text(i, x).\n.output defs\n";
    let program = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse(program).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let corpus = templates().join("corpus");
    std::fs::create_dir_all(&corpus).map_err(|e| e.to_string())?;
    let out = store.with_extension("out");
    let binding = crate::code_rule_dsl::typing::file_local::Binding::Cpu;
    super::run_with_options(&[&program], &corpus, None, &out, "program", binding, 1 << 18, &Default::default(), &serde_json::Value::Null).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&out);
    super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = None);
    let handle = crate::content_store::ContentStore::open(store).map_err(|e| e.to_string())?;
    handle.flush_atoms().map_err(|e| e.to_string())?;
    handle.written().map_err(|e| e.to_string())?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let policy = crate::content_store::CorpusPolicy::default();
    handle.keep_corpus_runs(&policy, now).map_err(|e| e.to_string())?;
    // A pass collects what is older than the store's generation, then advances it: two at least, then until one
    // finds nothing.
    for pass in 0.. {
        if !handle.collect_corpus_pass(&policy, now).map_err(|e| e.to_string())?.did_work() && pass >= 1 {
            break;
        }
    }
    Ok(())
}

fn rule_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            rule_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rr") {
            out.push(path);
        }
    }
}

/// A test's own store: its directory goes when it is dropped.
#[derive(Debug)]
pub struct Store {
    dir: PathBuf,
    path: PathBuf,
}

impl Store {
    /// The store's file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        // Its observer goes with it (`observe::forget`): the store's own handles have closed with the test's runs.
        crate::content_store::observe::forget(&crate::content_store::observe::store_beside(&self.path));
        crate::content_store::ContentStore::forget(&self.path);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// How many of this process's open files are under `dir` (macOS: `F_GETPATH`).
#[cfg(target_vendor = "apple")]
pub fn open_under(dir: &Path) -> usize {
    let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_owned());
    let prefix = format!("{}/", dir.display());
    (0..8192)
        .filter(|&fd| {
            let mut buf = [0u8; libc::PATH_MAX as usize];
            // SAFETY: F_GETPATH writes at most PATH_MAX bytes into `buf`.
            let named = unsafe { libc::fcntl(fd, libc::F_GETPATH, buf.as_mut_ptr()) } == 0;
            named && std::ffi::CStr::from_bytes_until_nul(&buf).is_ok_and(|p| p.to_string_lossy().starts_with(&prefix))
        })
        .count()
}

/// A copy of the template, in a directory of its own: a clone, which becomes a store of its own (its identity) at its
/// first open.
pub fn copy() -> Store {
    let template = template();
    static SWEPT: std::sync::Once = std::sync::Once::new();
    SWEPT.call_once(sweep_copies);
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = copies().join(format!("{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a test store's directory");
    let path = dir.join("store.sqlite");
    clone_file(template, &path).expect("a copy of the template");
    Store { dir, path }
}

thread_local! {
    /// This test's store (`own`), made on first use and removed when the test's thread ends.
    static OWN: std::cell::OnceCell<Store> = const { std::cell::OnceCell::new() };
}

/// The store of the test on this thread, made on first use. Off a test's thread (one the harness did not name after a
/// test: a pool's), `None`, said once per thread name.
pub(super) fn own() -> Option<PathBuf> {
    let thread = std::thread::current();
    let name = thread.name().unwrap_or("");
    if !name.contains("::") {
        static SAID: std::sync::Mutex<std::collections::BTreeSet<String>> = std::sync::Mutex::new(std::collections::BTreeSet::new());
        if SAID.lock().unwrap_or_else(|e| e.into_inner()).insert(name.to_owned()) {
            eprintln!("rerail: a store asked for off a test's thread ({name:?}): the process's");
        }
        return None;
    }
    Some(OWN.with(|own| own.get_or_init(copy).path.clone()))
}

/// Whether `path` is the store of the test on this thread (its own, not one shared with other threads).
pub(super) fn is_own(path: &Path) -> bool {
    OWN.with(|own| own.get().is_some_and(|s| s.path == path))
}

/// Removes the directories of test processes that have exited.
fn sweep_copies() {
    let me = std::process::id();
    for entry in std::fs::read_dir(copies()).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(pid) = name.split('-').next().and_then(|p| p.parse::<i32>().ok()) else { continue };
        // SAFETY: signal 0 sends nothing; it asks whether the process exists.
        if pid as u32 != me && unsafe { libc::kill(pid, 0) } != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Removes other builds' templates unused for `STALE`, and staging left by a process that died building one.
fn sweep_templates(dir: &Path, key: &str) {
    let now = std::time::SystemTime::now();
    let age = |path: &Path| std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| now.duration_since(t).ok());
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // By the template itself: one whose ready mark is gone is swept too, aged by its own time.
        let Some(other) = name.strip_suffix(".sqlite") else { continue };
        if other == key {
            continue;
        }
        let old = age(&dir.join(format!("{other}.ready"))).or_else(|| age(&entry.path())).is_some_and(|age| age > STALE);
        if old {
            for suffix in [".sqlite", ".ready", ".lock"] {
                let _ = std::fs::remove_file(dir.join(format!("{other}{suffix}")));
            }
        }
    }
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(pid) = name.split('.').nth(1).and_then(|p| p.parse::<i32>().ok()) else { continue };
        if !(name.ends_with(".build") || name.ends_with(".tmp")) {
            continue;
        }
        // SAFETY: as in `sweep_copies`.
        if unsafe { libc::kill(pid, 0) } != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            let path = entry.path();
            let _ = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        }
    }
}

/// `to`, a clone of `from` where the file system makes one (APFS), else a copy.
fn clone_file(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(target_vendor = "apple")]
    {
        use std::os::unix::ffi::OsStrExt as _;
        let a = std::ffi::CString::new(from.as_os_str().as_bytes())?;
        let b = std::ffi::CString::new(to.as_os_str().as_bytes())?;
        // SAFETY: both are NUL-terminated paths that live across the call.
        if unsafe { libc::clonefile(a.as_ptr(), b.as_ptr(), 0) } == 0 {
            return Ok(());
        }
    }
    std::fs::copy(from, to).map(|_| ())
}

fn nanos() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())
}

/// An exclusive lock on a file, across processes, until dropped.
struct Lock(std::fs::File);

impl Lock {
    fn take(path: &Path) -> Lock {
        use std::os::fd::AsRawFd as _;
        let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(path).expect("the template's lock");
        // SAFETY: the descriptor is the open file's, which outlives the lock.
        while unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::EINTR), "the template's lock");
        }
        Lock(file)
    }
}

#[cfg(test)]
mod tests {
    /// Another build's template unused for an hour goes, by its mark's time or, without one, its own; a fresh one and
    /// this build's stay.
    #[test]
    fn another_builds_template_unused_for_an_hour_is_swept() {
        let dir = tempfile::tempdir().unwrap();
        let at = |name: &str, ago: u64| {
            let file = std::fs::File::create(dir.path().join(name)).unwrap();
            file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(ago)).unwrap();
        };
        at("old.sqlite", 7200);
        at("old.lock", 7200);
        at("unmarked.sqlite", 7200);
        at("fresh.sqlite", 7200);
        at("fresh.ready", 60);
        at("mine.sqlite", 7200);
        super::sweep_templates(dir.path(), "mine");
        let left: std::collections::BTreeSet<String> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left, ["fresh.ready", "fresh.sqlite", "mine.sqlite"].map(String::from).into_iter().collect());
    }
}
