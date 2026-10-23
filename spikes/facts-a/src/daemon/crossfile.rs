//! The daemon's cross-file interface (PHASE-3.md § P3c): what depends on a worktree's membership,
//! not on one file. A clause is cross-file when it is a repository relational clause, a budget
//! (counted over the repository), or any clause of a rule with `report_when` (its findings are
//! reported only above a count over the whole evaluation); every other clause is per-file and the
//! daemon answers it synchronously.
//!
//! `CrossFile::apply` takes the worktree's membership after a change and the paths that changed,
//! and returns the findings delta. It runs on its own thread (`Worker`), never on the engine
//! thread: the engine hands it a snapshot of the membership with the epoch it was taken at, and
//! per-file replies never wait for it. Implementations:
//! - `FromScratch`: `eval::finish` over every member's stored per-file rows (the memo holds every
//!   evaluated clause's rows), i.e. the repository phase of `facts-a repo`; the reference any
//!   incremental model is compared with.
//! - P3a's model (facts-b's hybrid or compacted dataflow) plugs in here once it exposes an
//!   in-process entry point (docs/facts/PROTOCOL.md § Cross-file).
use crate::eval::{self, Failure, Finding, Kind, Rule};
use crate::memo::Memo;
use crate::spill::FileRows;
use rerail_engine::content_store::SourceId;
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Instant;

/// A member's content: its stored source (the store maps blake3 and Git blob hashes to it).
pub type Content = SourceId;

/// One path whose content changed (`None`: absent before / after).
pub struct PathChange {
    pub path: Arc<str>,
    pub old: Option<Content>,
    pub new: Option<Content>,
}

/// A worktree's current membership, as the cross-file side reads it.
pub trait Members {
    fn for_each(&self, f: &mut dyn FnMut(&str, Content));
}

#[derive(Default)]
pub struct FindingsDelta {
    pub introduced: Vec<Finding>,
    pub removed: Vec<Finding>,
    /// The cross-file clauses' failures, current (not a delta).
    pub failures: Vec<Failure>,
    /// Members whose stored rows were read (the work of a from-scratch run).
    pub files_read: u64,
}

pub trait CrossFile: Send {
    fn name(&self) -> &'static str;
    fn apply(&mut self, worktree: &str, members: &dyn Members, delta: &[PathChange]) -> FindingsDelta;
    /// Drops a closed worktree's state.
    fn forget(&mut self, worktree: &str);
    /// The worktrees' held cross-file findings (state proportional to findings).
    fn held_findings(&self) -> usize;
}

/// A worktree's membership at one epoch, as the engine hands it to the worker.
pub struct Snapshot(pub Vec<(Arc<str>, Content)>);

impl Members for Snapshot {
    fn for_each(&self, f: &mut dyn FnMut(&str, Content)) {
        for (p, c) in &self.0 {
            f(p, *c);
        }
    }
}

enum Msg {
    Apply { worktree: String, epoch: u64, members: Snapshot, changes: Vec<PathChange> },
    Forget(String),
}

/// The cross-file side's thread. The engine sends every membership change (`apply`) and close
/// (`forget`); the thread takes everything queued at once, keeps per worktree the latest snapshot
/// and every change since its last run, and runs `CrossFile::apply` once per worktree, pushing the
/// delta stamped with the snapshot's epoch. A burst of edits yields one run and one push per
/// worktree; a run in progress delays only later pushes.
pub struct Worker {
    tx: Option<mpsc::Sender<Msg>>,
    thread: Option<std::thread::JoinHandle<()>>,
    pub name: &'static str,
    held: Arc<AtomicUsize>,
    runs: Arc<AtomicU64>,
}

impl Worker {
    /// Starts the thread; `push(worktree, message)` delivers each push.
    pub fn spawn(mut cross: Box<dyn CrossFile>, push: Box<dyn Fn(&str, Json) + Send>) -> Worker {
        let name = cross.name();
        let (tx, rx) = mpsc::channel::<Msg>();
        let (held, runs) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicU64::new(0)));
        let (held2, runs2) = (held.clone(), runs.clone());
        let thread = std::thread::Builder::new()
            .name("facts-cross".into())
            .spawn(move || {
                while let Ok(first) = rx.recv() {
                    let mut pending: BTreeMap<String, (u64, Snapshot, Vec<PathChange>)> = BTreeMap::new();
                    for m in std::iter::once(first).chain(rx.try_iter()) {
                        match m {
                            Msg::Apply { worktree, epoch, members, changes } => match pending.get_mut(&worktree) {
                                Some(p) => {
                                    (p.0, p.1) = (epoch, members);
                                    p.2.extend(changes);
                                }
                                None => {
                                    pending.insert(worktree, (epoch, members, changes));
                                }
                            },
                            Msg::Forget(worktree) => {
                                pending.remove(&worktree);
                                cross.forget(&worktree);
                            }
                        }
                    }
                    for (worktree, (epoch, members, changes)) in pending {
                        let t0 = Instant::now();
                        let delta = cross.apply(&worktree, &members, &changes);
                        held2.store(cross.held_findings(), Ordering::Relaxed);
                        runs2.fetch_add(1, Ordering::Relaxed);
                        let row = |f: &Finding| json!([f.check, f.clause, f.path, f.line, f.code, f.column]);
                        push(
                            &worktree,
                            json!({
                                "op": "findings", "worktree": worktree, "epoch": epoch, "cross": true, "model": name,
                                "introduced": delta.introduced.iter().map(row).collect::<Vec<_>>(),
                                "removed": delta.removed.iter().map(row).collect::<Vec<_>>(),
                                "failures": delta.failures.iter().map(|f| json!([f.check, f.clause, f.path, f.message])).collect::<Vec<_>>(),
                                "members": members.0.len(), "changes": changes.len(),
                                "files_read": delta.files_read, "ms": t0.elapsed().as_secs_f64() * 1e3,
                            }),
                        );
                    }
                }
            })
            .expect("cross-file thread");
        Worker { tx: Some(tx), thread: Some(thread), name, held, runs }
    }

    pub fn apply(&self, worktree: &str, epoch: u64, members: Snapshot, changes: Vec<PathChange>) {
        let _ = self.tx.as_ref().expect("running").send(Msg::Apply { worktree: worktree.to_owned(), epoch, members, changes });
    }

    pub fn forget(&self, worktree: &str) {
        let _ = self.tx.as_ref().expect("running").send(Msg::Forget(worktree.to_owned()));
    }

    /// The worktrees' held cross-file findings, as of the last run.
    pub fn held_findings(&self) -> usize {
        self.held.load(Ordering::Relaxed)
    }

    pub fn runs(&self) -> u64 {
        self.runs.load(Ordering::Relaxed)
    }
}

impl Drop for Worker {
    /// Lets the thread finish the runs queued, then joins it.
    fn drop(&mut self) {
        drop(self.tx.take());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Per (rule, clause): whether the daemon delivers it synchronously (per-file) or through
/// `CrossFile`.
pub fn per_file_clauses(rules: &[Rule]) -> Vec<Vec<bool>> {
    rules
        .iter()
        .map(|r| {
            r.clauses
                .iter()
                .map(|c| {
                    r.report_count_gt.is_none()
                        && match &c.kind {
                            Kind::ForbidAst { .. } | Kind::ForbidRegex(_) => true,
                            Kind::Relational(p) => p.per_file,
                            Kind::Budget { .. } => false,
                        }
                })
                .collect()
        })
        .collect()
}

/// The from-scratch repository phase over the worktree's stored rows.
pub struct FromScratch {
    rules: Arc<[Rule]>,
    memo: Arc<Memo>,
    per_file: Vec<Vec<bool>>,
    /// Per worktree, its cross-file findings as a multiset.
    held: HashMap<String, BTreeMap<Key, u32>>,
}

type Key = (String, String, String, u32, String, Option<u32>);

fn key(f: &Finding) -> Key {
    (f.check.clone(), f.clause.clone(), f.path.clone(), f.line, f.code.clone(), f.column)
}

fn finding(k: &Key) -> Finding {
    Finding { check: k.0.clone(), clause: k.1.clone(), path: k.2.clone(), line: k.3, code: k.4.clone(), column: k.5 }
}

impl FromScratch {
    pub fn new(rules: Arc<[Rule]>, memo: Arc<Memo>) -> Self {
        let per_file = per_file_clauses(&rules);
        FromScratch { rules, memo, per_file, held: HashMap::new() }
    }

    /// The cross-file findings and failures of a membership, from scratch.
    pub fn evaluate(&self, members: &dyn Members) -> (Vec<Finding>, Vec<Failure>, u64) {
        let rules = &*self.rules;
        let mut out = eval::Output::default();
        let mut rows: BTreeMap<(usize, usize), Vec<FileRows>> = BTreeMap::new();
        let mut budget = eval::BudgetRows::new();
        let mut files = 0u64;
        members.for_each(&mut |path, content| {
            let scope: Vec<(usize, usize)> = crate::repo::in_scope(rules, path).into_iter().filter(|&(ri, ci)| !self.per_file[ri][ci]).collect();
            if scope.is_empty() {
                return;
            }
            files += 1;
            if !path.ends_with(".py") {
                // Text files: the gated rules' regex clauses, re-read (they are not memoised).
                if let Ok(bytes) = self.memo.store.get_source(content) {
                    if let Ok(text) = String::from_utf8(bytes) {
                        let mut o = eval::Output::default();
                        eval::eval_text_file(rules, path, &text, &mut o);
                        out.findings.extend(o.findings.into_iter().filter(|f| scope.iter().any(|&(ri, ci)| rules[ri].check == f.check && rules[ri].clauses[ci].id == f.clause)));
                    }
                }
                return;
            }
            let fm = self.memo.file_at(content, path);
            let rc: Rc<str> = Rc::from(path);
            for (ri, ci) in scope {
                let (rule, clause) = (&rules[ri], &rules[ri].clauses[ci]);
                // A member the memo lacks a clause for was not evaluated (it failed as a file): it
                // contributes nothing, as in `facts-a repo`.
                let Some(e) = fm.get(&rule.digests[ci], &rule.check, &clause.id, &rc) else { continue };
                out.findings.extend(e.findings);
                out.failures.extend(e.failures);
                if let Some(r) = e.rows {
                    rows.entry((ri, ci)).or_default().push(r);
                }
                if !e.budget.is_empty() {
                    budget.entry((ri, ci)).or_default().extend(e.budget);
                }
            }
        });
        eval::finish(rules, |k| rows.remove(&k).unwrap_or_default(), budget, &mut out);
        (out.findings, out.failures, files)
    }
}

impl CrossFile for FromScratch {
    fn name(&self) -> &'static str {
        "from-scratch"
    }

    fn apply(&mut self, worktree: &str, members: &dyn Members, _delta: &[PathChange]) -> FindingsDelta {
        let (findings, failures, files_read) = self.evaluate(members);
        let mut now: BTreeMap<Key, u32> = BTreeMap::new();
        for f in &findings {
            *now.entry(key(f)).or_default() += 1;
        }
        let before = self.held.remove(worktree).unwrap_or_default();
        let minus = |a: &BTreeMap<Key, u32>, b: &BTreeMap<Key, u32>| -> Vec<Finding> {
            a.iter().flat_map(|(k, n)| std::iter::repeat_n(k, n.saturating_sub(b.get(k).copied().unwrap_or(0)) as usize)).map(finding).collect()
        };
        let delta = FindingsDelta { introduced: minus(&now, &before), removed: minus(&before, &now), failures, files_read };
        self.held.insert(worktree.to_owned(), now);
        delta
    }

    fn forget(&mut self, worktree: &str) {
        self.held.remove(worktree);
    }

    fn held_findings(&self) -> usize {
        self.held.values().map(|m| m.values().sum::<u32>() as usize).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Holds its first run until released, and records each run's (worktree, changes).
    struct Gated {
        started: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
        runs: Arc<parking_lot::Mutex<Vec<(String, usize)>>>,
    }

    impl CrossFile for Gated {
        fn name(&self) -> &'static str {
            "gated"
        }
        fn apply(&mut self, worktree: &str, _: &dyn Members, delta: &[PathChange]) -> FindingsDelta {
            let first = self.runs.lock().is_empty();
            self.runs.lock().push((worktree.to_owned(), delta.len()));
            if first {
                self.started.send(()).unwrap();
                self.release.recv().unwrap();
            }
            FindingsDelta::default()
        }
        fn forget(&mut self, _: &str) {}
        fn held_findings(&self) -> usize {
            0
        }
    }

    fn change(path: &str) -> Vec<PathChange> {
        vec![PathChange { path: Arc::from(path), old: None, new: None }]
    }

    /// The worker runs off the caller's thread, and the changes queued during a run fold into one
    /// run per worktree, pushed with the latest epoch; a forgotten worktree's queued change is not
    /// run.
    #[test]
    fn queued_changes_fold_into_one_run_per_worktree() {
        let (started_tx, started) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let runs = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let pushes = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let p = pushes.clone();
        let worker = Worker::spawn(
            Box::new(Gated { started: started_tx, release: release_rx, runs: runs.clone() }),
            Box::new(move |w, push| p.lock().push((w.to_owned(), push["epoch"].as_u64().unwrap(), push["changes"].as_u64().unwrap()))),
        );
        worker.apply("a", 1, Snapshot(Vec::new()), change("x.py"));
        started.recv().unwrap();
        // The first run is in progress: these queue behind it, and the caller is not blocked.
        worker.apply("a", 2, Snapshot(Vec::new()), change("y.py"));
        worker.apply("b", 1, Snapshot(Vec::new()), change("z.py"));
        worker.apply("a", 3, Snapshot(Vec::new()), change("x.py"));
        worker.apply("c", 1, Snapshot(Vec::new()), change("w.py"));
        worker.forget("c");
        release.send(()).unwrap();
        drop(worker);
        assert_eq!(*runs.lock(), vec![("a".to_owned(), 1), ("a".to_owned(), 2), ("b".to_owned(), 1)]);
        assert_eq!(*pushes.lock(), vec![("a".to_owned(), 1, 1), ("a".to_owned(), 3, 2), ("b".to_owned(), 1, 1)]);
    }
}
