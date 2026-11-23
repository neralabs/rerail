//! A long-lived corpus runner (decision 0063): what a process that runs the same program again and again keeps from
//! one run to the next, so that a run after an edit costs what the edit changed. The browser's engine is one: the
//! explorer runs a rule over the files on its page at each keystroke, in one instance of the engine.
//!
//! A session is its store, kept open, and nothing else of its own (t33 step 3): its runs are the store's runs, as a
//! new process's are. They keep their kept facts there (each file's memo entry, the cross-file part's relations and
//! their statistics, the corpus batch's documents' table) and read them where the store keeps them, mapped once a
//! process while the store's chain holds (`kept_read::Bound`); an edit evaluates the files it changed and maintains
//! the cross-file part from its kept rows (t33 K4). A session keeps:
//!
//! - its store (`session.sqlite`), which binding it selects for its runs (`ThreadStore`): atoms are never given out
//!   again (decision 0049), and its kept facts are under them;
//! - its connections (`connections`), kept from run to run: an edit's run opens none;
//! - nothing of the program itself: decision 0060's caches keep it for the process.
//!
//! A run with T's facts or typing is not a session's (`corpus::run_with_options`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(in crate::code_rule_dsl) struct Session {
    /// The store's file, which its runs keep their facts in (`Handle::bind` selects it).
    path: PathBuf,
    /// Its runs' connections and stores, kept from run to run (`connections`): an edit's run opens none.
    connections: Arc<parking_lot::Mutex<super::connections::Connections>>,
}

static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A session, owned by whoever keeps it from run to run: a long-lived process (the browser's engine, `repeat`) keeps
/// one, a test opens its own. Nothing about it is the process's: two sessions run side by side, each with its store.
#[derive(Clone)]
pub struct Handle {
    /// Its runs' kept plans and derived values are taken back only by its own runs. Outside the lock, so a run can
    /// name its session before it holds it.
    generation: u64,
    session: Arc<parking_lot::Mutex<Session>>,
}

impl Handle {
    /// A new session, its store in `dir`.
    pub fn open(dir: &Path) -> std::result::Result<Handle, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = dir.join("session.sqlite");
        let generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let session = Session { path, connections: Arc::new(parking_lot::Mutex::new(super::connections::Connections::kept())) };
        Ok(Handle { generation, session: Arc::new(parking_lot::Mutex::new(session)) })
    }

    /// Makes the corpus runs this thread starts, until the guard is dropped, this session's runs.
    pub fn bind(&self) -> Bound {
        let path = self.session.lock().path.clone();
        let _session = crate::code_rule_dsl::logic::profile::bind(InSession(Some(self.clone()), super::ThreadStore::current()));
        Bound { _store: super::ThreadStore::select(&path, None), _session }
    }
}

/// A session bound to this thread's runs (`Handle::bind`); dropped, the thread's runs are as they were before.
pub struct Bound {
    // Dropped first: bound last (guards drop in the reverse of their order, decision 0442).
    _store: super::ThreadStore,
    _session: crate::code_rule_dsl::logic::profile::Bound,
}

/// The session this caller's runs are runs of, in the run's context: a job's nested runs are runs of its caller's
/// session (decision 0192), as they were when the session was the process's.
/// With the store selected outside it, which a full run uses (`full`).
#[derive(Clone, Default)]
struct InSession(Option<Handle>, super::SelectedStore);

/// A full run (`full`), in the run's context: a job's runs are full where its caller's are.
#[derive(Clone, Copy, Default)]
struct Full(bool);

/// Opens a session in `dir` and binds it to this thread's runs: they are its runs until the guard is dropped.
pub fn start(dir: &Path) -> std::result::Result<Bound, String> {
    Handle::open(dir).map(|handle| handle.bind())
}

/// The session `RERAIL_SESSION` names (decision 0063), kept by a caller that runs one command after another
/// (`tools::cli`'s callers): the same directory keeps its session, another one opens a new one.
#[derive(Default)]
pub struct Kept(Option<(std::ffi::OsString, Handle)>);

impl Kept {
    /// The session for the next command, bound to this thread until the guard is dropped: none when
    /// `RERAIL_SESSION` is not set.
    pub fn bind_from_env(&mut self) -> std::result::Result<Option<Bound>, String> {
        let Some(dir) = std::env::var_os("RERAIL_SESSION") else { return Ok(None) };
        if self.0.as_ref().is_none_or(|(kept, _)| *kept != dir) {
            self.0 = Some((dir.clone(), Handle::open(Path::new(&dir))?));
        }
        Ok(self.0.as_ref().map(|(_, handle)| handle.bind()))
    }
}

/// The generation of the session this thread's next run is a run of (`None`: no session, or a full run).
pub(super) fn generation() -> Option<u64> {
    if super::phase0::active() || crate::code_rule_dsl::logic::profile::bound::<Full>().0 {
        return None;
    }
    crate::code_rule_dsl::logic::profile::with_bound(|s: &InSession| s.0.as_ref().map(|h| h.generation))
}

/// Runs `run` as a full run: as a process without a session would, compiling the program and evaluating every file,
/// so its profile shows every step (`logic --full`). The session is left as it was.
pub fn full<T>(run: impl FnOnce() -> T) -> T {
    // On the store selected outside the session: the session's store is its runs' alone.
    let outside = crate::code_rule_dsl::logic::profile::bound::<InSession>().0.is_some().then(|| super::ThreadStore::again(crate::code_rule_dsl::logic::profile::bound::<InSession>().1));
    let _full = crate::code_rule_dsl::logic::profile::bind(Full(true));
    let value = run();
    drop(_full);
    drop(outside);
    value
}

/// The session, held for a run (`None`: none bound to this thread, or a full run). Only this session's runs wait
/// for one another; a run of no session holds nothing.
pub(super) fn current() -> Held {
    // Phase 0's runs (`phase0`) are never a session's, and may run inside one of the session's runs.
    if super::phase0::active() || crate::code_rule_dsl::logic::profile::bound::<Full>().0 {
        return Held(None);
    }
    let Some(handle) = crate::code_rule_dsl::logic::profile::bound::<InSession>().0 else { return Held(None) };
    // A run holds its session for all of it, so another run of the same session (a job's: the typing programs beside
    // phase 1, decision 0192) waits for it to end. On a pool thread the wait helps the pool (decision 0100 §12):
    // blocked, the thread held the stolen parts of the holder's own parallel work under it, and neither ended.
    let held = if rerail_accel::exec::on_pool() {
        rerail_accel::exec::help_until_as("the session, held by another run", || handle.session.try_lock_arc())
    } else {
        handle.session.lock_arc()
    };
    Held(Some(held))
}

/// The session held for a run.
pub(super) struct Held(Option<parking_lot::ArcMutexGuard<parking_lot::RawMutex, Session>>);

impl Held {
    /// Whether the run is a session's.
    pub(super) fn is_some(&self) -> bool {
        self.0.is_some()
    }
    /// The session's connections, which its run takes and hands back.
    pub(super) fn connections(&self) -> Option<Arc<parking_lot::Mutex<super::connections::Connections>>> {
        self.0.as_deref().map(|s| Arc::clone(&s.connections))
    }
}

// Its tests run in the engine's test binary (its facade of this module mounts `session/tests.rs`): they read what the
// engine registers as it loads (the languages, their `.rerail` libraries).

/// An output stack as a run left it: its file's length and modification time, its images, the rows its overlays
/// added or took away since its base, and each relation's rows' hashes as it reads them (`output::Rel::hash`).
#[derive(Clone, Debug)]
pub(super) struct Stack {
    pub len: u64,
    pub modified: std::time::SystemTime,
    pub depth: usize,
    pub deltas: usize,
    pub rels: std::collections::HashMap<String, Arc<Vec<u128>>>,
}

impl Stack {
    /// The stack in `out` as a stat finds it (no image of it known).
    pub fn stat(out: &std::path::Path) -> Option<Stack> {
        let m = std::fs::metadata(out.join(crate::code_rule_dsl::logic::plan::output::FILE)).ok()?;
        Some(Stack { len: m.len(), modified: m.modified().ok()?, depth: 0, deltas: 0, rels: Default::default() })
    }
    /// The stack in `out` as its file holds it (t33 step 3: read, where a session kept it in memory): its depth, the
    /// rows its overlays added, took away or wrote whole since its base, and each relation's rows' hashes as it reads
    /// them. `None` where `out` holds no stack.
    pub fn read(out: &std::path::Path) -> Option<Stack> {
        use crate::code_rule_dsl::logic::plan::output;
        let stat = Stack::stat(out)?;
        let outputs = output::Outputs::open(out).ok()?;
        let images = outputs.images();
        let names = outputs.names();
        let deltas = images.iter().skip(1).map(|image| {
            names.iter().map(|n| [output::kind::WHOLE, output::kind::ADDED, output::kind::GONE].iter().filter_map(|k| image.entry(n, *k)).map(|r| r.rows).sum::<usize>()).sum::<usize>()
        }).sum();
        let rels = names.iter().filter_map(|n| Some(((*n).to_owned(), Arc::new(output::find(images, n)?.hashes())))).collect();
        Some(Stack { depth: images.len(), deltas, rels, ..stat })
    }
}
