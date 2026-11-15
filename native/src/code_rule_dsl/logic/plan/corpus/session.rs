//! A long-lived corpus runner (decision 0063): what a process that runs the same program again and again keeps from
//! one run to the next, so that a run after an edit costs what the edit changed. The browser's engine is one: the
//! explorer runs a rule over the files on its page at each keystroke, in one instance of the engine.
//!
//! A session keeps:
//!
//! - its store, so atoms are never given out again (decision 0049) and keep their meaning from run to run: the
//!   compiled program's constants and the memoised rows are under them;
//! - nothing of the program itself: decision 0060's caches keep it for the process (`combine` by its texts, the
//!   prepared program by its texts and additions, its atoms checked against this store), so an edit to a file
//!   neither parses nor lowers the rule again;
//! - each file's file-local rows (decision 0052's memo), in memory, under its content, path and the file-local
//!   program: an unchanged file is neither parsed nor evaluated.
//!
//! The cross-file part reuses unchanged retained strata. Gates reuse only a complete unchanged program.
//! A run with T's facts or typing is not a session's
//! (`corpus::run_with_options`).

use super::memo;
use crate::content_store::ContentStore;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub(in crate::code_rule_dsl) struct Session {
    /// The store's file, which each run's SQL connection opens (its atoms are the store's).
    pub(super) path: PathBuf,
    pub(super) store: ContentStore,
    pub(super) memo: memo::Held,
    /// The cross-file part as the last run left it (decision 0077): an edit runs the strata whose inputs changed. One
    /// per cross program (`retain::key`): a run's passes are different programs, and one would take the other's place.
    pub(super) cross: Mutex<std::collections::HashMap<[u8; 32], super::retain::Retained>>,
    /// Each output file a run of the session wrote: the program's cross-file key and its rows. A later run of that
    /// program whose output stratum is clean leaves the file as it is.
    pub(super) written: Mutex<std::collections::HashMap<PathBuf, ([u8; 32], usize)>>,
    /// Each memoised program's base relations' statistics, kept by file (decision 0096 §3, `keptstats`).
    pub(super) stats: Mutex<std::collections::HashMap<[u8; 32], super::keptstats::Keeper>>,
    /// Its runs' connections and stores, kept from run to run (`connections`): an edit's run opens none.
    pub(super) connections: Arc<parking_lot::Mutex<super::connections::Connections>>,
    /// Its runs' chunk mixes' tree counts (`own_counts`), read from memory as they are written behind the runs.
    pub(super) own: super::OwnKept,
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
        let store = ContentStore::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let session = Session {
            path,
            store,
            memo: Default::default(),
            cross: Default::default(),
            written: Mutex::new(Default::default()),
            stats: Mutex::new(Default::default()),
            connections: Arc::new(parking_lot::Mutex::new(super::connections::Connections::kept())),
            own: Default::default(),
        };
        Ok(Handle { generation, session: Arc::new(parking_lot::Mutex::new(session)) })
    }

    /// Makes the corpus runs this thread starts, until the guard is dropped, this session's runs.
    pub fn bind(&self) -> Bound {
        let before = BOUND.with(|b| b.replace(Some(self.clone())));
        Bound { before }
    }
}

/// A session bound to this thread's runs (`Handle::bind`); dropped, the thread's runs are as they were before.
pub struct Bound {
    before: Option<Handle>,
}

impl Drop for Bound {
    fn drop(&mut self) {
        let before = self.before.take();
        BOUND.with(|b| *b.borrow_mut() = before);
    }
}

thread_local! {
    /// The session this thread's runs are runs of.
    static BOUND: std::cell::RefCell<Option<Handle>> = const { std::cell::RefCell::new(None) };
    /// A full run (`full`) on this thread.
    static FULL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

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
    if super::phase0::active() || FULL.with(std::cell::Cell::get) {
        return None;
    }
    BOUND.with(|b| b.borrow().as_ref().map(|h| h.generation))
}

/// Runs `run` as a full run: as a process without a session would, compiling the program and evaluating every file,
/// so its profile shows every step (`logic --full`). The session is left as it was.
pub fn full<T>(run: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            FULL.with(|f| f.set(self.0));
        }
    }
    let _reset = Reset(FULL.with(|f| f.replace(true)));
    run()
}

/// The session, held for a run (`None`: none bound to this thread, or a full run). Only this session's runs wait
/// for one another; a run of no session holds nothing.
pub(super) fn current() -> Held {
    // Phase 0's runs (`phase0`) are never a session's, and may run inside one of the session's runs.
    if super::phase0::active() || FULL.with(std::cell::Cell::get) {
        return Held(None);
    }
    Held(BOUND.with(|b| b.borrow().as_ref().map(|h| h.session.lock_arc())))
}

/// The session held for a run.
pub(super) struct Held(Option<parking_lot::ArcMutexGuard<parking_lot::RawMutex, Session>>);

impl Held {
    pub(super) fn as_ref(&self) -> Option<&Session> {
        self.0.as_deref()
    }
    pub(super) fn as_mut(&mut self) -> Option<&mut Session> {
        self.0.as_deref_mut()
    }
}

// Its tests run in the engine's test binary (its facade of this module mounts `session/tests.rs`): they read what the
// engine registers as it loads (the languages, their `.rr` libraries).
