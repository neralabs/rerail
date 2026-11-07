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
//! The cross-file part runs every time, over every file's rows. A run with T's facts or typing is not a session's
//! (`corpus::run_with_options`).

use super::memo;
use crate::content_store::ContentStore;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

pub(in crate::code_rule_dsl) struct Session {
    /// The store's file, which each run's SQL connection opens (its atoms are the store's).
    pub(super) path: PathBuf,
    pub(super) store: ContentStore,
    pub(super) memo: memo::Held,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

/// Starts this process's session, its store in `dir`; one already started is kept.
pub(in crate::code_rule_dsl) fn start(dir: &Path) -> std::result::Result<(), String> {
    let mut session = SESSION.lock().map_err(|_| "the session is poisoned".to_owned())?;
    if session.is_some() {
        return Ok(());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join("session.sqlite");
    let store = ContentStore::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    *session = Some(Session { path, store, memo: Default::default() });
    Ok(())
}

/// Ends this process's session: later runs are a fresh process's.
#[cfg(test)]
pub(in crate::code_rule_dsl) fn stop() {
    *SESSION.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

static FULL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Runs `run` as a full run: as a process without a session would, compiling the program and evaluating every file,
/// so its profile shows every step (`logic --full`). The session is left as it was.
pub(in crate::code_rule_dsl) fn full<T>(run: impl FnOnce() -> T) -> T {
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
    Held(SESSION.lock().unwrap_or_else(std::sync::PoisonError::into_inner), FULL.load(std::sync::atomic::Ordering::Relaxed))
}

/// The session held for a run, unless the run is a full one.
pub(super) struct Held(MutexGuard<'static, Option<Session>>, bool);

impl Held {
    pub(super) fn as_ref(&self) -> Option<&Session> {
        self.0.as_ref().filter(|_| !self.1)
    }
    pub(super) fn as_mut(&mut self) -> Option<&mut Session> {
        self.0.as_mut().filter(|_| !self.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::typing::file_local::Binding;

    /// Two runs of one program in a session: the second compiles nothing and reads the unchanged file from the memo;
    /// a third, after an edit, evaluates the edited file only, and every run's findings are a fresh run's.
    #[test]
    fn a_session_reruns_what_an_edit_changed() {
        let base = std::env::temp_dir().join(format!("rerail-session-{}", std::process::id()));
        let (src, out) = (base.join("src"), base.join("out"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("a.py"), "import hashlib\nhashlib.md5(b'x')\n").unwrap();
        std::fs::write(src.join("b.py"), "import pickle\npickle.loads(b'x')\n").unwrap();
        let rule = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/packs/python/bandit-b301-pickle.rr");
        let program = crate::code_rule_dsl::tools::flatten(&rule).unwrap();
        let run = || super::super::run(&program, &src, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
        let findings = || std::fs::read_to_string(out.join("violation.csv")).unwrap();
        let fresh = run();
        let expected = findings();
        let _ = fresh;
        start(&base.join("session")).unwrap();
        let first = run();
        assert_eq!((first.memo.0, findings()), (0, expected.clone()), "a session's first run evaluates every file");
        let second = run();
        assert_eq!((second.memo.0, findings()), (2, expected.clone()), "an unchanged corpus is read from the memo");
        std::fs::write(src.join("a.py"), "import pickle\nimport hashlib\nhashlib.md5(b'x')\npickle.loads(b'y')\n").unwrap();
        let third = run();
        assert_eq!(third.memo.0, 1, "only the edited file is evaluated");
        assert!(findings().lines().count() == expected.lines().count() + 1, "the edit's finding: {}", findings());
        stop();
        let _ = std::fs::remove_dir_all(&base);
    }
}
