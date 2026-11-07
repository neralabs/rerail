//! A long-lived corpus runner (decision 0056): what a process that runs the same program again and again keeps from
//! one run to the next, so that a run after an edit costs what the edit changed. The browser's engine is one: the
//! explorer runs a rule over the files on its page at each keystroke, in one instance of the engine.
//!
//! A session keeps:
//!
//! - its store, so atoms are never given out again (decision 0049) and keep their meaning from run to run: the
//!   compiled program's constants and the memoised rows are under them;
//! - the last program combined (`combine`), by its texts, and compiled (`Compiled`), by those texts and the inputs
//!   the corpus settled (`external_inputs`): an edit to a file neither parses nor lowers the rule again;
//! - each file's file-local rows (decision 0052's memo), in memory, under its content, path and the file-local
//!   program: an unchanged file is neither parsed nor evaluated.
//!
//! The cross-file part runs every time, over every file's rows. A run with T's facts or typing is not a session's
//! (`corpus::run_with_options`).

use super::{Compiled, Result, memo};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::logic::front;
use crate::content_store::ContentStore;
use sha2::{Digest as _, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

pub(in crate::code_rule_dsl) struct Session {
    /// The store's file, which each run's SQL connection opens (its atoms are the store's).
    pub(super) path: PathBuf,
    pub(super) store: ContentStore,
    pub(super) memo: memo::Held,
    /// The last program combined, by a digest of its texts.
    combined: Mutex<Option<([u8; 32], front::Program)>>,
    /// The last program compiled, by its texts' digest, its settled inputs and its dialect.
    compiled: Mutex<Option<([u8; 32], Arc<Compiled>)>>,
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
    *session = Some(Session { path, store, memo: Default::default(), combined: Mutex::new(None), compiled: Mutex::new(None) });
    Ok(())
}

/// Ends this process's session: later runs are a fresh process's.
#[cfg(test)]
pub(in crate::code_rule_dsl) fn stop() {
    *current() = None;
}

/// The session, held for a run (`None`: none started).
pub(super) fn current() -> MutexGuard<'static, Option<Session>> {
    SESSION.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Session {
    /// `programs` combined (`corpus::combine`): the last program's again when the texts are the same.
    pub(super) fn combine(&self, programs: &[&str]) -> Result<front::Program> {
        let mut hash = Sha256::new();
        for p in programs {
            hash.update((p.len() as u64).to_le_bytes());
            hash.update(p.as_bytes());
        }
        let key: [u8; 32] = hash.finalize().into();
        let mut last = self.combined.lock().expect("the session's program");
        if let Some((k, prog)) = &*last
            && *k == key
        {
            crate::code_rule_dsl::logic::profile::decision("session", || "the program as combined last run (its texts are the same)".into());
            return Ok(prog.clone());
        }
        let prog = super::combine(programs, "")?;
        *last = Some((key, prog.clone()));
        Ok(prog)
    }

    /// `prog`, the program `combine` gave with its inputs settled, compiled: the last compiled again when it is the
    /// same program over the same inputs and dialect, stating the decisions compiling made.
    pub(super) fn compiled(&self, prog: front::Program, dialect: GrammarDialect, store: &ContentStore) -> Result<Arc<Compiled>> {
        let mut hash = Sha256::new();
        hash.update(self.combined.lock().expect("the session's program").as_ref().map_or([0; 32], |(k, _)| *k));
        hash.update(format!("{:?}\n{:?}\n{:?}", prog.inputs, prog.facts, crate::code_rule_dsl::language::label(dialect)).as_bytes());
        let key: [u8; 32] = hash.finalize().into();
        let mut last = self.compiled.lock().expect("the session's compiled program");
        if let Some((k, compiled)) = &*last
            && *k == key
        {
            crate::code_rule_dsl::logic::profile::add_decisions(&compiled.decisions);
            crate::code_rule_dsl::logic::profile::decision("session", || "the program as compiled last run: not simplified or lowered again".into());
            return Ok(Arc::clone(compiled));
        }
        let compiled = Arc::new(Compiled::of(prog, dialect, store)?);
        // The previous program's rows are under its own file-local digest: no run reads them again.
        self.memo.lock().expect("the session memo").clear();
        *last = Some((key, Arc::clone(&compiled)));
        Ok(compiled)
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
        assert_eq!((first.memo_hits, findings()), (0, expected.clone()), "a session's first run evaluates every file");
        let second = run();
        assert_eq!((second.memo_hits, findings()), (2, expected.clone()), "an unchanged corpus is read from the memo");
        std::fs::write(src.join("a.py"), "import pickle\nimport hashlib\nhashlib.md5(b'x')\npickle.loads(b'y')\n").unwrap();
        let third = run();
        assert_eq!(third.memo_hits, 1, "only the edited file is evaluated");
        assert!(findings().lines().count() == expected.lines().count() + 1, "the edit's finding: {}", findings());
        stop();
        let _ = std::fs::remove_dir_all(&base);
    }
}
