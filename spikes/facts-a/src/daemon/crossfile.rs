//! The daemon's cross-file interface (PHASE-3.md § P3c): what depends on a worktree's membership,
//! not on one file. A clause is cross-file when it is a repository relational clause, a budget
//! (counted over the repository), or any clause of a rule with `report_when` (its findings are
//! reported only above a count over the whole evaluation); every other clause is per-file and the
//! daemon answers it synchronously.
//!
//! `CrossFile::apply` takes the worktree's membership after a change and the paths that changed,
//! and returns the findings delta. Implementations:
//! - `FromScratch`: `eval::finish` over every member's stored per-file rows (the memo holds every
//!   evaluated clause's rows), i.e. the repository phase of `facts-a repo`; the reference any
//!   incremental model is compared with.
//! - P3a's model (facts-b's hybrid or compacted dataflow) plugs in here once it exposes an
//!   in-process entry point (docs/facts/PROTOCOL.md § Cross-file).
use crate::eval::{self, Failure, Finding, Kind, Rule};
use crate::memo::Memo;
use crate::spill::FileRows;
use rerail_engine::content_store::SourceId;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;

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

pub trait CrossFile {
    fn name(&self) -> &'static str;
    fn apply(&mut self, worktree: &str, members: &dyn Members, delta: &[PathChange]) -> FindingsDelta;
    /// Drops a closed worktree's state.
    fn forget(&mut self, worktree: &str);
    /// The worktrees' held cross-file findings (state proportional to findings).
    fn held_findings(&self) -> usize;
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
