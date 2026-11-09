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
    /// In a test build, the thread that started the session, the only one whose runs are its: the other tests run
    /// beside it, in the same process, and are no session's.
    #[cfg(test)]
    thread: std::thread::ThreadId,
    pub(super) memo: memo::Held,
    /// The cross-file part as the last run left it (decision 0077): an edit runs the strata whose inputs changed.
    pub(super) cross: Mutex<Option<super::retain::Retained>>,
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
    *session = Some(Session {
        path,
        store,
        memo: Default::default(),
        cross: Mutex::new(None),
        #[cfg(test)]
        thread: std::thread::current().id(),
    });
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
    let held = SESSION.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    #[cfg(test)]
    let skip = held.as_ref().is_some_and(|s| s.thread != std::thread::current().id());
    #[cfg(not(test))]
    let skip = false;
    Held(held, skip || FULL.load(std::sync::atomic::Ordering::Relaxed))
}

/// The session held for a run, unless the run is a full one.
pub(super) struct Held(MutexGuard<'static, Option<Session>>, bool);

impl Held {
    pub(super) fn as_ref(&self) -> Option<&Session> {
        self.0.as_ref().filter(|s| !self.1 && Self::ours(s))
    }
    pub(super) fn as_mut(&mut self) -> Option<&mut Session> {
        let full = self.1;
        self.0.as_mut().filter(|s| !full && Self::ours(s))
    }
    /// Whether a run on this thread is the session's: always, outside tests.
    fn ours(_session: &Session) -> bool {
        #[cfg(test)]
        return _session.thread == std::thread::current().id();
        #[cfg(not(test))]
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::typing::file_local::Binding;

    /// The process has one session: its tests take turns.
    static SERIAL: Mutex<()> = Mutex::new(());

    /// Two runs of one program in a session: the second compiles nothing and reads the unchanged file from the memo;
    /// a third, after an edit, evaluates the edited file only, and every run's findings are a fresh run's.
    #[test]
    fn a_session_reruns_what_an_edit_changed() {
        let _session = super::super::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let _one = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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

    /// A session keeps its cross-file part (decision 0077): a run over unchanged files evaluates no cross-file stratum,
    /// a run after an edit evaluates those whose inputs changed, and every run's findings are a full run's.
    #[test]
    fn a_session_runs_the_cross_file_strata_an_edit_changed() {
        let _one = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let base = std::env::temp_dir().join(format!("rerail-session-cross-{}", std::process::id()));
        let (src, out) = (base.join("src"), base.join("out"));
        std::fs::create_dir_all(src.join("pkg")).unwrap();
        std::fs::write(src.join("pkg/__init__.py"), "").unwrap();
        std::fs::write(src.join("pkg/loader.py"), "import pickle as p\n\ndef load(b):\n    return p.loads(b)\n").unwrap();
        std::fs::write(src.join("main.py"), "from pkg.loader import load\nload(b'x')\n").unwrap();
        std::fs::write(src.join("other.py"), "import json\njson.loads('1')\n").unwrap();
        let rules = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/packs/python");
        let programs: Vec<String> = ["bandit-b301-pickle.rr", "bandit-b307-eval.rr"].iter().map(|r| crate::code_rule_dsl::tools::flatten(&rules.join(r)).unwrap()).collect();
        let texts: Vec<&str> = programs.iter().map(String::as_str).collect();
        let run = || super::super::run_all(&texts, &src, &out, "program", Binding::Cpu, 1 << 18).unwrap();
        let findings = || {
            let mut rows: Vec<String> = std::fs::read_to_string(out.join("violation.csv")).unwrap_or_default().lines().map(str::to_owned).collect();
            rows.sort();
            rows
        };
        let full = || {
            let report = full(run);
            assert_eq!(report.cross_strata, None, "a full run is no session's");
            findings()
        };
        start(&base.join("session")).unwrap();
        let first = run();
        let (ran, all) = first.cross_strata.expect("a session's run");
        assert_eq!(ran, all, "the first run evaluates every stratum");
        assert!(all > 0, "the rules have a cross-file part");
        let expected = full();
        assert_eq!(findings(), expected);
        let again = run();
        assert_eq!(again.cross_strata, Some((0, all)), "nothing changed: no stratum runs");
        assert_eq!(findings(), expected);
        let edits: [(&str, &str); 3] = [
            ("other.py", "import json\nimport pickle\njson.loads('1')\npickle.loads(b'2')\n"),
            ("main.py", "from pkg.loader import load\nload(b'x')\neval('1')\n"),
            ("pkg/loader.py", "import marshal as p\n\ndef load(b):\n    return p.loads(b)\n"),
        ];
        let mut maintained = 0;
        for (path, text) in edits {
            std::fs::write(src.join(path), text).unwrap();
            let report = run();
            let got = findings();
            assert_eq!(got, full(), "after editing {path}, the session's findings are a full run's");
            let (ran, _) = report.cross_strata.expect("a session's run");
            assert!(ran <= all, "{path}: {ran} of {all}");
            eprintln!("{path}: {ran} of {all} strata run, {} maintained from deltas ({} recursive); {} findings", report.cross_maintained.0, report.cross_maintained.1, got.len());
            maintained += report.cross_maintained.0;
        }
        assert!(maintained > 0, "an edit's strata are maintained from deltas (stage 2)");
        stop();
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A recursive cross-file stratum (import reachability) is maintained by delete-and-rederive (decision 0077 §3):
    /// an edit that breaks a cycle deletes the rows that lost every derivation, one that closes it again inserts them,
    /// and every run's findings are a full run's.
    #[test]
    fn a_session_maintains_a_recursive_cross_file_stratum() {
        let _one = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let base = std::env::temp_dir().join(format!("rerail-session-reach-{}", std::process::id()));
        let (src, out) = (base.join("src"), base.join("out"));
        std::fs::create_dir_all(&src).unwrap();
        let rule = base.join("reach.rr");
        std::fs::write(
            &rule,
            r#".policy "test-import-cycle" revision 1
.description "An import that closes a cycle of imports."
.language python
.mode steer
.scope include "**/*.py"
#include "python/names.rr"
.decl dep(a: number, b: number)
dep(a, b) :- impmod(i, m), docof(i, a), doc(b, _, m).
.decl reach(a: number, b: number)
reach(a, b) :- dep(a, b).
reach(a, c) :- reach(a, b), dep(b, c).
.violation CYC(at: node) code "CYC" "This import closes a cycle of imports."
CYC(i) :- impmod(i, m), docof(i, a), doc(b, _, m), reach(b, a).
"#,
        )
        .unwrap();
        for (path, text) in [("a.py", "from b import x\n"), ("b.py", "from c import y\n"), ("c.py", "from a import z\n"), ("d.py", "x = 1\n")] {
            std::fs::write(src.join(path), text).unwrap();
        }
        let program = crate::code_rule_dsl::tools::flatten(&rule).unwrap();
        let run = || super::super::run(&program, &src, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
        let findings = || {
            let mut rows: Vec<String> = std::fs::read_to_string(out.join("violation.csv")).unwrap_or_default().lines().map(str::to_owned).collect();
            rows.sort();
            rows
        };
        let full = || {
            full(run);
            findings()
        };
        start(&base.join("session")).unwrap();
        run();
        assert_eq!(findings(), full());
        assert!(findings().len() >= 3, "the cycle's imports: {:?}", findings());
        let edits: [(&str, &str); 4] = [
            ("c.py", "from d import z\n"),
            ("d.py", "from a import w\n"),
            ("b.py", "x = 2\n"),
            ("b.py", "from c import y\n"),
        ];
        let mut recursive = 0;
        for (path, text) in edits {
            std::fs::write(src.join(path), text).unwrap();
            let report = run();
            let got = findings();
            assert_eq!(got, full(), "after editing {path}, the session's findings are a full run's");
            eprintln!("{path}: {:?} strata run, {:?} maintained (recursive); {} findings", report.cross_strata, report.cross_maintained, got.len());
            recursive += report.cross_maintained.1;
        }
        assert!(recursive > 0, "the recursive stratum is maintained");
        stop();
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A cross-file stratum with aggregates is maintained by its changed groups (decision 0077 §3): a module's
    /// importers counted, up and down to none, give a full run's findings.
    #[test]
    fn a_session_maintains_a_cross_file_aggregate() {
        let _one = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let base = std::env::temp_dir().join(format!("rerail-session-count-{}", std::process::id()));
        let (src, out) = (base.join("src"), base.join("out"));
        std::fs::create_dir_all(&src).unwrap();
        let rule = base.join("hub.rr");
        std::fs::write(
            &rule,
            r#".policy "test-import-hub" revision 1
.description "An import of a module two files import, and an import of a module that imports nothing."
.language python
.mode steer
.scope include "**/*.py"
#include "python/names.rr"
.decl dep(a: number, b: number)
dep(a, b) :- impmod(i, m), docof(i, a), doc(b, _, m).
.decl fan(b: number, n: number)
fan(b, n) :- doc(b, _, _), n = count : { dep(_, b) }.
.decl leaf(b: number)
leaf(b) :- doc(b, _, _), count : { dep(b, _) } = 0.
.violation HUB(at: node) code "HUB" "An import of a module two or more files import."
HUB(i) :- impmod(i, m), doc(b, _, m), fan(b, n), n >= 2.
.violation LEAF(at: node) code "LEAF" "An import of a module that imports nothing."
LEAF(i) :- impmod(i, m), doc(b, _, m), leaf(b).
"#,
        )
        .unwrap();
        for (path, text) in [("a.py", "from c import x\n"), ("b.py", "from c import y\n"), ("c.py", "from d import z\n"), ("d.py", "x = 1\n")] {
            std::fs::write(src.join(path), text).unwrap();
        }
        let program = crate::code_rule_dsl::tools::flatten(&rule).unwrap();
        let run = || super::super::run(&program, &src, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
        let findings = || {
            let mut rows: Vec<String> = std::fs::read_to_string(out.join("violation.csv")).unwrap_or_default().lines().map(str::to_owned).collect();
            rows.sort();
            rows
        };
        let full = || {
            full(run);
            findings()
        };
        start(&base.join("session")).unwrap();
        run();
        assert_eq!(findings(), full());
        assert!(findings().iter().any(|r| r.contains("HUB")) && findings().iter().any(|r| r.contains("LEAF")), "{:?}", findings());
        let edits: [(&str, &str); 4] = [
            ("b.py", "x = 2\n"),
            ("d.py", "from a import w\n"),
            ("b.py", "from c import y\nfrom d import q\n"),
            ("d.py", "x = 1\n"),
        ];
        let mut maintained = 0;
        for (path, text) in edits {
            std::fs::write(src.join(path), text).unwrap();
            let report = run();
            let got = findings();
            assert_eq!(got, full(), "after editing {path}, the session's findings are a full run's");
            eprintln!("{path}: {:?} strata run, {:?} maintained (recursive, derived again); {} findings", report.cross_strata, report.cross_maintained, got.len());
            assert_eq!(report.cross_maintained.2, 0, "no stratum is derived again");
            maintained += report.cross_maintained.0;
        }
        assert!(maintained > 0);
        stop();
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A negation with a wildcard, `!dep(a, _)`, over a relation that loses one of two rows: the negation still fails
    /// (spike O125). Delete-and-rederive's insert rule must test it over the new rows, not take the lost row for it.
    #[test]
    fn a_session_keeps_a_negation_that_still_fails() {
        let _one = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let base = std::env::temp_dir().join(format!("rerail-session-lone-{}", std::process::id()));
        let (src, out) = (base.join("src"), base.join("out"));
        std::fs::create_dir_all(&src).unwrap();
        let rule = base.join("lone.rr");
        std::fs::write(
            &rule,
            r#".policy "test-unresolved-import" revision 1
.description "An import of no file, in a file that imports none."
.language python
.mode steer
.scope include "**/*.py"
#include "python/names.rr"
.decl dep(a: number, b: number)
dep(a, b) :- impmod(i, m), docof(i, a), doc(b, _, m).
.violation LONE(at: node) code "LONE" "An import of no file, in a file that imports none."
LONE(i) :- impmod(i, m), docof(i, a), !dep(a, _).
"#,
        )
        .unwrap();
        for (path, text) in [("a.py", "from b import x\nfrom c import y\nfrom zz import z\n"), ("b.py", "x = 1\n"), ("c.py", "y = 2\n"), ("d.py", "from zz import z\n")] {
            std::fs::write(src.join(path), text).unwrap();
        }
        let program = crate::code_rule_dsl::tools::flatten(&rule).unwrap();
        let run = || super::super::run(&program, &src, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
        let findings = || {
            let mut rows: Vec<String> = std::fs::read_to_string(out.join("violation.csv")).unwrap_or_default().lines().map(str::to_owned).collect();
            rows.sort();
            rows
        };
        let full = || {
            full(run);
            findings()
        };
        start(&base.join("session")).unwrap();
        run();
        assert_eq!(findings(), full());
        let mut maintained = 0;
        for (path, text) in [("a.py", "from c import y\nfrom zz import z\n"), ("a.py", "from zz import z\n"), ("a.py", "from b import x\nfrom zz import z\n")] {
            std::fs::write(src.join(path), text).unwrap();
            let report = run();
            assert_eq!(findings(), full(), "after editing {path} to {text:?}, the session's findings are a full run's");
            maintained += report.cross_maintained.0;
        }
        assert!(maintained > 0, "the negation's stratum is maintained");
        stop();
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Where a session's run spends its CPU (decision 0090, step 1): each linear cost's span, for a one-file edit
    /// over `RERAIL_SESSION_COST_FILES` modules (default 400) in an import chain, with reachability and counts.
    /// A measurement, not a check: `cargo test … session_costs -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn session_costs() {
        let _one = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let n: usize = std::env::var("RERAIL_SESSION_COST_FILES").ok().and_then(|v| v.parse().ok()).unwrap_or(400);
        let base = std::env::temp_dir().join(format!("rerail-session-costs-{}", std::process::id()));
        let (src, out) = (base.join("src"), base.join("out"));
        std::fs::create_dir_all(&src).unwrap();
        let rule = base.join("costs.rr");
        std::fs::write(
            &rule,
            r#".policy "test-session-costs" revision 1
.description "Imports that close a cycle, and imports of modules many files import."
.language python
.mode steer
.scope include "**/*.py"
#include "python/names.rr"
.decl dep(a: number, b: number)
dep(a, b) :- impmod(i, m), docof(i, a), doc(b, _, m).
.decl reach(a: number, b: number)
reach(a, b) :- dep(a, b).
reach(a, c) :- reach(a, b), dep(b, c).
.decl fan(b: number, n: number)
fan(b, n) :- doc(b, _, _), n = count : { dep(_, b) }.
.violation CYC(at: node) code "CYC" "An import that closes a cycle."
CYC(i) :- impmod(i, m), docof(i, a), doc(b, _, m), reach(b, a).
.violation HUB(at: node) code "HUB" "An import of a module four or more files import."
HUB(i) :- impmod(i, m), doc(b, _, m), fan(b, k), k >= 4.
"#,
        )
        .unwrap();
        // Module i imports i+1 and i/7: a chain, with hubs, so reach is quadratic in a window only.
        let module = |i: usize, extra: &str| format!("from m{} import x\nfrom m{} import y\n{extra}def f{i}(a):\n    return a\n", (i + 1) % (n + 1), i / 7);
        for i in 0..n {
            std::fs::write(src.join(format!("m{i}.py")), module(i, "")).unwrap();
        }
        let program = crate::code_rule_dsl::tools::flatten(&rule).unwrap();
        let run = || super::super::run(&program, &src, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
        crate::code_rule_dsl::logic::profile::set(true);
        let show = |what: &str, report: &super::super::Report| {
            let spans = crate::code_rule_dsl::logic::profile::spans();
            let cross: f64 = spans.iter().filter(|(p, _)| p.ends_with("/cross") || p.as_str() == "cross").map(|(_, t)| t.total).sum();
            eprintln!("{what}: strata {:?}, maintained {:?}; cross {cross:.3} s CPU", report.cross_strata, report.cross_maintained);
            for (path, t) in spans.iter().filter(|(p, _)| p.split_once("session/").is_some_and(|(_, rest)| !rest.contains('/'))) {
                eprintln!("    {:>7.3} s  {path}", t.total);
            }
            // The costliest strata, and their parts: loading inputs and indexes, and the joins.
            let mut strata: Vec<(&String, f64)> = spans.iter().filter(|(p, _)| p.contains("session/plan and evaluate/stratum ") && p.matches('/').count() == 3).map(|(p, t)| (p, t.total)).collect();
            strata.sort_by(|a, b| b.1.total_cmp(&a.1));
            // `RERAIL_SESSION_COST_SPANS`: every span of the session's run past 0.1 s, at any depth.
            if std::env::var_os("RERAIL_SESSION_COST_SPANS").is_some() {
                for (path, t) in spans.iter().filter(|(p, t)| p.contains("session/") && t.total > 0.1) {
                    eprintln!("    {:>7.3} s  {}", t.total, path.split_once("session/").map_or(path.as_str(), |(_, r)| r));
                }
            }
            for (path, total) in strata.into_iter().take(12) {
                let part = |name: &str| spans.get(&format!("{path}/native/{name}")).map_or(0.0, |t| t.total);
                eprintln!("    {total:>7.3} s  {}: open {:.3} s, joins {:.3} s", path.rsplit('/').next().unwrap_or(""), part("engine open (load inputs, build indexes)"), part("engine run (joins)"));
            }
        };
        start(&base.join("session")).unwrap();
        let r = run();
        show("first run", &r);
        for (k, (i, extra)) in [(n / 2, "z = 1\n"), (n / 3, "from m0 import q\n"), (n / 2, "")].into_iter().enumerate() {
            std::fs::write(src.join(format!("m{i}.py")), module(i, extra)).unwrap();
            let r = run();
            show(&format!("edit {k} (m{i})"), &r);
        }
        stop();
        let r = run();
        show("cold run, no session", &r);
        crate::code_rule_dsl::logic::profile::set(false);
        let _ = std::fs::remove_dir_all(&base);
    }
}
