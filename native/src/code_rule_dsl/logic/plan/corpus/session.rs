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
use std::sync::{Arc, Mutex, MutexGuard};

pub(in crate::code_rule_dsl) struct Session {
    /// The store's file, which each run's SQL connection opens (its atoms are the store's).
    pub(super) path: PathBuf,
    pub(super) store: ContentStore,
    /// In a test build, the thread that started the session, the only one whose runs are its: the other tests run
    /// beside it, in the same process, and are no session's.
    #[cfg(any(test, feature = "test-hooks"))]
    thread: std::thread::ThreadId,
    pub(super) memo: memo::Held,
    /// The cross-file part as the last run left it (decision 0077): an edit runs the strata whose inputs changed. One
    /// per cross program (`retain::key`): a run's passes are different programs, and one would take the other's place.
    pub(super) cross: Mutex<std::collections::HashMap<[u8; 32], super::retain::Retained>>,
    /// Each output file a run of the session wrote: the program's cross-file key and its rows. A later run of that
    /// program whose output stratum is clean leaves the file as it is.
    pub(super) written: Mutex<std::collections::HashMap<PathBuf, ([u8; 32], usize)>>,
    /// Each memoised program's base relations' statistics, kept by file (decision 0096 §3, `keptstats`).
    pub(super) stats: Mutex<std::collections::HashMap<[u8; 32], super::keptstats::Keeper>>,
    /// Which session this is: its runs' kept plans and derived values are taken back only by its own runs.
    generation: u64,
    /// Its runs' connections and stores, kept from run to run (`connections`): an edit's run opens none.
    pub(super) connections: Arc<parking_lot::Mutex<super::connections::Connections>>,
    /// Its runs' chunk mixes' tree counts (`own_counts`), read from memory as they are written behind the runs.
    pub(super) own: super::OwnKept,
}


static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// The generation of the session this thread's next run is a run of (`None`: no session, or a full run).
pub(super) fn generation() -> Option<u64> {
    current().as_ref().map(|s| s.generation)
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

/// Starts this process's session, its store in `dir`; one already started is kept.
pub fn start(dir: &Path) -> std::result::Result<(), String> {
    let mut session = SESSION.lock().map_err(|_| "the session is poisoned".to_owned())?;
    if session.is_some() {
        return Ok(());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join("session.sqlite");
    let store = ContentStore::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    *session = Some(Session {
        path,
        store,
        memo: Default::default(),
        cross: Default::default(),
        written: Mutex::new(Default::default()),
        stats: Mutex::new(Default::default()),
        generation: GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        connections: Arc::new(parking_lot::Mutex::new(super::connections::Connections::kept())),
        own: Default::default(),
        #[cfg(any(test, feature = "test-hooks"))]
        thread: std::thread::current().id(),
    });
    Ok(())
}

/// Ends this process's session: later runs are a fresh process's.
#[cfg(any(test, feature = "test-hooks"))]
pub fn stop() {
    *SESSION.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

static FULL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Runs `run` as a full run: as a process without a session would, compiling the program and evaluating every file,
/// so its profile shows every step (`logic --full`). The session is left as it was.
pub fn full<T>(run: impl FnOnce() -> T) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            FULL.store(false, std::sync::atomic::Ordering::Relaxed);
        }
    }
    FULL.store(true, std::sync::atomic::Ordering::Relaxed);
    let _reset = Reset;
    run()
}

/// The session, held for a run (`None`: none started, or a full run).
pub(super) fn current() -> Held {
    // Phase 0's runs (`phase0`) are never a session's, and may run while another thread's run holds it.
    if super::phase0::active() {
        return Held(None, true);
    }
    let held = SESSION.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    #[cfg(any(test, feature = "test-hooks"))]
    let skip = held.as_ref().is_some_and(|s| s.thread != std::thread::current().id());
    #[cfg(not(any(test, feature = "test-hooks")))]
    let skip = false;
    Held(Some(held), skip || FULL.load(std::sync::atomic::Ordering::Relaxed))
}

/// The session held for a run, unless the run is a full one.
pub(super) struct Held(Option<MutexGuard<'static, Option<Session>>>, bool);

impl Held {
    pub(super) fn as_ref(&self) -> Option<&Session> {
        self.0.as_ref()?.as_ref().filter(|s| !self.1 && Self::ours(s))
    }
    pub(super) fn as_mut(&mut self) -> Option<&mut Session> {
        let full = self.1;
        self.0.as_mut()?.as_mut().filter(|s| !full && Self::ours(s))
    }
    /// Whether a run on this thread is the session's: always, outside tests.
    fn ours(_session: &Session) -> bool {
        #[cfg(any(test, feature = "test-hooks"))]
        return _session.thread == std::thread::current().id();
        #[cfg(not(any(test, feature = "test-hooks")))]
        true
    }
}

#[cfg(test)]
pub mod tests;
