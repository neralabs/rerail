//! One file's evaluation for the daemon: every in-scope clause's findings and failures for a
//! (path, content), served by the memo where it holds the clause's plan digest and evaluated
//! otherwise (the fresh entries are stored, repository and budget rows included, so the cross-file
//! side reads them from the store). Evaluation splits the missing clauses by rule over `threads`
//! workers, each with its own facts build (facts hold per-file lazy caches and are not shared
//! between threads); one thread evaluates them in rule order.
use crate::eval::{self, Failure, Finding, Rule};
use crate::memo::{Encoded, Entry, Memo};
use crate::{facts, repo, spill};
use rerail_engine::content_store::SourceId;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// One clause's per-file result as the daemon keeps it.
pub struct Reported {
    pub clause: (usize, usize),
    pub findings: Vec<Finding>,
    pub failures: Vec<Failure>,
}

/// What one (path, content) evaluation produced.
#[derive(Default)]
pub struct Evaluated {
    /// Every in-scope clause's result, by (rule, clause).
    pub reported: Vec<Reported>,
    /// Failures of the whole file (not UTF-8, not parsed), with no clause.
    pub file_failures: Vec<Failure>,
    /// Clause runs evaluated and served by the memo.
    pub evaluated: u32,
    pub memoised: u32,
}

/// Evaluates `path` holding the stored content `source`. `bytes` is called only if some clause
/// misses the memo.
pub fn evaluate(rules: &[Rule], memo: &Memo, path: &str, source: SourceId, bytes: &dyn Fn() -> Vec<u8>, threads: usize) -> Evaluated {
    let mut done = Evaluated::default();
    if !path.ends_with(".py") {
        // A file the facts do not cover (`.rs`): its regex clauses read the text, unmemoised (as in
        // `facts-a repo`).
        let Ok(text) = String::from_utf8(bytes()) else {
            done.file_failures.push(Failure { check: String::new(), clause: String::new(), path: path.to_owned(), message: "file could not be read as UTF-8".into() });
            return done;
        };
        let mut out = eval::Output::default();
        eval::eval_text_file(rules, path, &text, &mut out);
        let mut by: BTreeMap<(usize, usize), Vec<Finding>> = BTreeMap::new();
        for f in out.findings {
            by.entry(clause_key(rules, &f.check, &f.clause)).or_default().push(f);
        }
        done.evaluated = by.len() as u32;
        done.reported = by.into_iter().map(|(clause, findings)| Reported { clause, findings, failures: Vec::new() }).collect();
        return done;
    }
    let scope = repo::in_scope(rules, path);
    if scope.is_empty() {
        return done;
    }
    let mut fm = memo.file_at(source, path);
    let mut misses: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for &(ri, ci) in &scope {
        let (rule, clause) = (&rules[ri], &rules[ri].clauses[ci]);
        match fm.get_reported(&rule.digests[ci], &rule.check, &clause.id, path) {
            Some((findings, failures)) => {
                done.reported.push(Reported { clause: (ri, ci), findings, failures });
                done.memoised += 1;
            }
            None => misses.entry(ri).or_default().push(ci),
        }
    }
    if misses.is_empty() {
        return done;
    }
    let Ok(text) = String::from_utf8(bytes()) else {
        done.file_failures.push(Failure { check: String::new(), clause: String::new(), path: path.to_owned(), message: "file is not UTF-8".into() });
        return done;
    };
    let text = Arc::new(text);
    let tasks: Vec<(usize, Vec<usize>)> = misses.into_iter().collect();
    let next = AtomicUsize::new(0);
    let workers = threads.clamp(1, tasks.len());
    type Fresh = Vec<(Reported, Encoded)>;
    let run = || -> Result<Fresh, ()> {
        let mut fresh = Vec::new();
        let mut built: Option<facts::Facts> = None;
        loop {
            let i = next.fetch_add(1, Ordering::Relaxed);
            let Some((ri, cis)) = tasks.get(i) else { break };
            if built.is_none() {
                match facts::build(path, text.clone()) {
                    Ok(facts::Built::Facts(f)) => built = Some(f),
                    Ok(facts::Built::Unparsed) | Err(_) => return Err(()),
                }
            }
            for ((ri, ci), e) in eval_rule(rules, *ri, cis, built.as_ref().unwrap(), path, &text) {
                let encoded = memo.encode(ri, ci, &e, path);
                fresh.push((Reported { clause: (ri, ci), findings: e.findings, failures: e.failures }, encoded));
            }
        }
        Ok(fresh)
    };
    let results: Vec<Result<Fresh, ()>> = std::thread::scope(|s| {
        let handles: Vec<_> = (1..workers).map(|_| s.spawn(&run)).collect();
        let mut all = vec![run()];
        all.extend(handles.into_iter().map(|h| h.join().expect("evaluation thread")));
        all
    });
    if results.iter().any(Result::is_err) {
        done.file_failures.push(Failure { check: String::new(), clause: String::new(), path: path.to_owned(), message: "file could not be parsed".into() });
        return done;
    }
    for (r, encoded) in results.into_iter().flat_map(Result::unwrap) {
        let (ri, ci) = r.clause;
        fm.put_encoded(ri, ci, &rules[ri].digests[ci], encoded);
        done.reported.push(r);
        done.evaluated += 1;
    }
    fm.save(text.as_bytes());
    done.reported.sort_by_key(|r| r.clause);
    done
}

/// Rule `ri`'s clauses `cis` on one file: each clause's entry, as `facts-a repo` stores it.
fn eval_rule(rules: &[Rule], ri: usize, cis: &[usize], f: &facts::Facts, path: &str, text: &str) -> Vec<((usize, usize), Entry)> {
    let rule = &rules[ri];
    let one = std::slice::from_ref(rule);
    let mut out = eval::Output { file_counts: Some(Vec::new()), ..Default::default() };
    out.skip = (0..rule.clauses.len()).filter(|ci| !cis.contains(ci)).map(|ci| (0, ci)).collect();
    let mut sink = spill::Memory::default();
    let mut budget = eval::BudgetRows::new();
    eval::eval_file(one, &eval::FileCtx { f, path, source: text }, &mut out, &mut sink, &mut budget);
    let counts = out.file_counts.take().unwrap_or_default();
    cis.iter()
        .map(|&ci| {
            let id = &rule.clauses[ci].id;
            let e = Entry {
                findings: out.findings.iter().filter(|x| &x.clause == id).cloned().collect(),
                failures: out.failures.iter().filter(|x| &x.clause == id).cloned().collect(),
                counts: counts.iter().filter(|(k, _)| *k == (0, ci)).fold(Default::default(), |mut a, (_, c)| {
                    a += c;
                    a
                }),
                rows: sink.0.remove(&(0, ci)).and_then(|mut v| v.pop()),
                budget: budget.remove(&(0, ci)).unwrap_or_default(),
            };
            ((ri, ci), e)
        })
        .collect()
}

/// The (rule, clause) a finding names.
fn clause_key(rules: &[Rule], check: &str, clause: &str) -> (usize, usize) {
    rules
        .iter()
        .enumerate()
        .find(|(_, r)| r.check == check)
        .and_then(|(ri, r)| r.clauses.iter().position(|c| c.id == clause).map(|ci| (ri, ci)))
        .expect("a finding names a compiled clause")
}
