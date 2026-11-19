//! Each test's own store, copied from a template (the data-layer repository's SQLite fixtures, `conform/conftest.py`).
//!
//! A test that does not name a store (`TEST_TAPE_CACHE`, `RERAIL_TAPE_CACHE`) gets one of its own on first use: a
//! copy of the template (its file and its packs), cloned (APFS `clonefile`, else a copy) into a directory of its own,
//! and removed when the test's thread ends. No test sees what another left behind, and none waits on another's leases
//! (O146).
//!
//! The template holds what every test would otherwise make first: the rule library's phase 0 (each module's tape and
//! its loads, `bootstrap.rerail`'s plan) and the core program's plan. It is made once per build (keyed by the build's
//! identities, `build_ids`), by the first test process that wants it, under a file lock; the processes after it find
//! it ready (`<key>.ready`). It is written to a staging store, drained (atoms, queued writes), and published with
//! `VACUUM INTO`, its memo packs beside it (decision 0240 §4), and a rename, so a reader never sees half of one. A
//! binary that registers no language (a lower crate's) cannot run phase 0: its template is an empty store, and it says
//! so once.
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

thread_local! {
    /// This thread fills the template (`make`): its runs take the switches without making it first.
    static FILLING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether this thread fills the template (`make`). What its runs keep for the process (phase 0's graphs) the process's
/// own runs would find made, and skip the work a test of them compares (`phase0::graphs`).
pub(super) fn filling() -> bool {
    FILLING.with(std::cell::Cell::get)
}

/// The template made, before a test's thread takes the switches (`switches::reading`, `switches::set`): no thread waits
/// for it while it holds them, so its runs never wait behind a thread that waits for it. Off a test's thread (`own`:
/// a pool's, or a process the tests start) nothing reads the template, so nothing makes it.
pub(super) fn ready() {
    if !FILLING.with(std::cell::Cell::get) && std::thread::current().name().is_some_and(|n| n.contains("::")) {
        template();
    }
}

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
        .spawn(move || {
            // Its runs skip `ready` (they fill the template), and take nothing the thread that waits on them holds.
            FILLING.with(|f| f.set(true));
            crate::code_rule_dsl::logic::profile::behind(|| fill(&filling))
        })
        .expect("the template's thread")
        .join()
        .unwrap_or_else(|panic| Err(panic.downcast_ref::<String>().cloned().or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()));
    if let Err(why) = filled {
        eprintln!("rerail: the test stores' template holds no library ({why}); every test's store starts empty");
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging).expect("the template's staging directory");
        drop(crate::content_store::ContentStore::open(&store).expect("an empty store"));
    }
    publish(&store, path);
    let _ = std::fs::remove_dir_all(&staging);
}

/// Publishes the drained store at `store` as the template at `path`: its file whole, then renamed into place.
fn publish(store: &Path, path: &Path) {
    let published = path.with_extension(format!("{}.{}.tmp", std::process::id(), nanos()));
    let db = rusqlite::Connection::open(store).expect("the template's staging store");
    db.execute("VACUUM INTO ?1", [published.to_string_lossy()]).expect("the template, written whole");
    drop(db);
    // Published as a copy needs it: in WAL mode (a store's first open would switch it), and with no identity (each
    // copy takes its own at its first open, `ContentStore::initialize`), so that a copy is a clone and nothing more.
    let db = rusqlite::Connection::open(&published).expect("the published template");
    db.execute_batch("DELETE FROM store_identity; PRAGMA journal_mode=WAL;").expect("the template, made ready to copy");
    drop(db);
    // The memo's entries are in packs beside the store (decision 0240 §4), which its rows name: published with it,
    // before the file that names them. Without them a copy's rows named a pack its own writer began again, and read
    // its new entries' bytes as theirs.
    let packs = crate::content_store::memo_pack_dir(path);
    let _ = std::fs::remove_dir_all(&packs);
    copy_packs(&crate::content_store::memo_pack_dir(store), &packs).expect("the template's packs, published");
    std::fs::rename(&published, path).expect("the template, published");
}

/// Phase 0 over every library module and the core program, then a run of the core program over an empty corpus:
/// what a test's first run would do before it reads its own files. Then collected, as a run's collection would leave
/// it: a process whose include graphs were already in memory (the template is made by a test, mid-suite) writes
/// `core.rerail`'s tape with no manifest naming it, and a copy's first collection, due ten minutes after the template's
/// decision, took it out from under the test (`a_tests_store_is_a_copy_of_the_template_and_goes_with_its_test`).
fn fill(store: &Path) -> std::result::Result<(), String> {
    super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(store.to_owned()));
    let lib = dunce::canonicalize(crate::code_rule_dsl::logic::rr::library()).map_err(|e| e.to_string())?;
    let mut seeds = vec![super::phase0::Seed::Text { t_shape: false, includes: vec![(0, "core.rerail".to_owned())], phases: Vec::new() }];
    let mut modules = Vec::new();
    rule_files(&lib, &mut modules);
    modules.sort();
    seeds.extend(modules.into_iter().map(super::phase0::Seed::File));
    super::phase0::graphs(&seeds).map_err(|e| e.to_string())?;
    let program = "#include \"core.rerail\"\n.decl defs(s: symbol)\ndefs(x) :- kind(n, \"function_definition\"), field(n, \"name\", i), text(i, x).\n.output defs\n";
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
        } else if path.extension().is_some_and(|e| e == "rerail") {
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
    let dir = dunce::canonicalize(dir).unwrap_or_else(|_| dir.to_owned());
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
    clone_store(template, &path);
    Store { dir, path }
}

/// The template at `template` cloned to `path`: its file and its packs.
fn clone_store(template: &Path, path: &Path) {
    clone_file(template, path).expect("a copy of the template");
    copy_packs(&crate::content_store::memo_pack_dir(template), &crate::content_store::memo_pack_dir(path)).expect("a copy of the template's packs");
}

thread_local! {
    /// This test's store (`own`), made on first use and removed when the test's thread ends.
    static OWN: std::cell::OnceCell<Store> = const { std::cell::OnceCell::new() };
}

thread_local! {
    /// The store of the test whose run a job on this thread works for (`carried`).
    static HANDED: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Carries the test's store to a job on another thread (`run_state`): the job's runs use the store of the test that
/// started it, not the process's.
pub(in crate::code_rule_dsl) fn carried(c: &mut crate::code_rule_dsl::logic::run_state::Capture) {
    let named = std::thread::current().name().is_some_and(|n| n.contains("::"));
    // The store this thread's runs use (`tape_cache_path`'s order): the one its test names (`TEST_TAPE_CACHE`), else
    // the one it was handed, else the process's when it names one (`RERAIL_TAPE_CACHE`, `use_store`: the job's runs
    // find it themselves), else its own. Handed its own over the process's, a job wrote a fresh copy where the test's
    // runs read the named store.
    let mine = super::TEST_TAPE_CACHE
        .with(|p| p.borrow().clone())
        .or_else(handed)
        .or_else(|| (named && !super::names_its_store()).then(|| OWN.with(|own| own.get_or_init(copy).path.clone())));
    c.part(&["OWN", "HANDED"], crate::code_rule_dsl::logic::run_state::Shared(mine, hand));
}

/// The store this thread was handed (`carried`), if any.
pub(super) fn handed() -> Option<PathBuf> {
    HANDED.with(|h| h.borrow().clone())
}

/// While the guard lives, this thread's runs use `store` (another thread's test's).
fn hand(store: Option<PathBuf>) -> impl Drop {
    struct Handed(Option<PathBuf>);
    impl Drop for Handed {
        fn drop(&mut self) {
            HANDED.with(|h| h.replace(self.0.take()));
        }
    }
    Handed(HANDED.with(|h| h.replace(store)))
}

/// The store of the test on this thread, made on first use; on a job's thread, the store of the test that started it
/// (`carried`). Off a test's thread (one the harness did not name after a test: a pool's), `None`, said once per
/// thread name.
pub(super) fn own() -> Option<PathBuf> {
    if let Some(handed) = HANDED.with(|h| h.borrow().clone()) {
        return Some(handed);
    }
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
        if pid as u32 != me && super::process_gone(pid) {
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
            let _ = std::fs::remove_dir_all(crate::content_store::memo_pack_dir(&entry.path()));
        }
    }
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(pid) = name.split('.').nth(1).and_then(|p| p.parse::<i32>().ok()) else { continue };
        if !(name.ends_with(".build") || name.ends_with(".tmp")) {
            continue;
        }
        if super::process_gone(pid) {
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

/// The packs of the store at `from` (`content_store::memo_pack_dir`) into `to`, each a clone where it can be: none
/// where `from` has none.
fn copy_packs(from: &Path, to: &Path) -> std::io::Result<()> {
    let Ok(packs) = std::fs::read_dir(from) else { return Ok(()) };
    std::fs::create_dir_all(to)?;
    for pack in packs {
        let pack = pack?;
        clone_file(&pack.path(), &to.join(pack.file_name()))?;
    }
    Ok(())
}

fn nanos() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())
}

/// An exclusive lock on a file, across processes, until dropped.
struct Lock(std::fs::File);

impl Lock {
    /// std's file lock: `flock` on Unix, `LockFileEx` on Windows; it ends when the file closes.
    fn take(path: &Path) -> Lock {
        let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(path).expect("the template's lock");
        file.lock().expect("the template's lock");
        Lock(file)
    }
}

#[cfg(test)]
mod tests {
    /// Another build's template unused for an hour goes, by its mark's time or, without one, its own, and its packs
    /// with it; a fresh one and this build's stay.
    #[test]
    fn another_builds_template_unused_for_an_hour_is_swept() {
        let dir = tempfile::tempdir().unwrap();
        let at = |name: &str, ago: u64| {
            let file = std::fs::File::create(dir.path().join(name)).unwrap();
            file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(ago)).unwrap();
        };
        at("old.sqlite", 7200);
        at("old.lock", 7200);
        std::fs::create_dir_all(dir.path().join("old.sqlite.memo")).unwrap();
        at("old.sqlite.memo/1.pack", 7200);
        at("unmarked.sqlite", 7200);
        at("fresh.sqlite", 7200);
        at("fresh.ready", 60);
        std::fs::create_dir_all(dir.path().join("fresh.sqlite.memo")).unwrap();
        at("mine.sqlite", 7200);
        super::sweep_templates(dir.path(), "mine");
        let left: std::collections::BTreeSet<String> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left, ["fresh.ready", "fresh.sqlite", "fresh.sqlite.memo", "mine.sqlite"].map(String::from).into_iter().collect());
    }

    /// A template is published, and copied, with its memo packs: the copy's rows find the template's bytes (decision
    /// 0240 §4), and its own writes go after them. Its file alone was published and copied before, so its rows named a
    /// pack the copy's writer began again, and read the copy's entries as theirs
    /// (`a_store_copied_without_its_packs_reads_no_other_entrys_bytes`).
    #[test]
    fn a_template_is_published_and_copied_with_its_memo_packs() {
        use crate::content_store::{ContentStore, MemoPut};
        let dir = tempfile::tempdir().unwrap();
        let space = crate::content_store::local_space(&[7u8; 32]);
        let put = |part: i64, byte: u8| MemoPut { part, path: format!("{part}.py"), digest: [part as u8; 32], key: 1, bytes: vec![byte; 100], over: false };
        let staging = dir.path().join("staging.sqlite");
        ContentStore::open(&staging).unwrap().put_memo_entries(&space, (0..4).map(|p| put(p, 1)).collect(), 4).unwrap();
        let template = dir.path().join("template.sqlite");
        super::publish(&staging, &template);
        let copy = dir.path().join("copy/store.sqlite");
        std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
        super::clone_store(&template, &copy);
        let store = ContentStore::open(&copy).unwrap();
        store.put_memo_entries(&space, (4..8).map(|p| put(p, 2)).collect(), 4).unwrap();
        let read = store.memo_entries(&space, &(0..8).collect::<Vec<_>>(), true).unwrap();
        assert_eq!(read.len(), 8, "the template's entries and the copy's each find their bytes");
        for (part, blob) in read {
            assert!(blob.bytes.bytes().iter().all(|&b| b == if part < 4 { 1 } else { 2 }), "part {part} reads the bytes it was put with");
        }
    }
}
