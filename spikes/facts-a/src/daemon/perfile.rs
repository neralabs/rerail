//! The per-file evaluation of a request's (path, content) pairs: every in-scope clause's findings
//! and failures, served by the memo where it holds the clause's plan digest and evaluated otherwise
//! (the fresh entries are stored, repository and budget rows included, so the cross-file side
//! reads them from the store).
//!
//! Two phases over the daemon's evaluation workers (one persistent pool: regex match caches are
//! per thread, so threads spawned per request would each add their own). First every pair's memo
//! lookups (and a text file's regex clauses, unmemoised); the caller gets those results at once.
//! Then the clauses the memo missed, as one queue of tasks over every pair of the request, the
//! largest files first: a task is a whole pair when there are at least two pairs per worker, else
//! one rule's missed clauses on one pair, split into runs when a few large files leave workers
//! idle (a large file's clauses spread over the workers). A worker builds a pair's facts once for
//! the tasks it takes of that pair (facts hold per-file lazy caches and are not shared between
//! threads). The caller's thread evaluates nothing: it hands each batch of finished rule runs to
//! the caller as the workers deliver them, and stores a pair's fresh entries when its last run is
//! in.
use crate::eval::{self, Failure, Finding, Rule};
use crate::memo::{Encoded, Entry, FileMemo, Memo};
use crate::{facts, repo, spill};
use rayon::prelude::*;
use rerail_engine::content_store::SourceId;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;

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

/// One pair to evaluate: its path, its stored content, and its bytes when the request read them.
pub struct Job {
    pub path: Arc<str>,
    pub source: SourceId,
    pub bytes: Option<Arc<Vec<u8>>>,
}

/// What `evaluate` hands its caller while it runs; each result is (job index, clause result).
pub enum Progress<'a> {
    /// The clauses the memo served (and text files' regex clauses), before any clause is
    /// evaluated. `evaluating`: some clause missed the memo and evaluation follows.
    Served { results: &'a [(usize, Reported)], evaluating: bool },
    /// The clauses the workers finished since the previous call.
    Evaluated(&'a [(usize, Reported)]),
}

/// The evaluation's shape: tasks queued and workers' scratch.
#[derive(Default, Clone, Copy)]
pub struct Shape {
    pub tasks: usize,
    pub workers: usize,
    /// The largest net heap one worker held during the evaluation (jemalloc `thread.peak`).
    pub scratch_peak: u64,
}

/// A pair after its memo lookups.
struct Looked<'m> {
    fm: Option<FileMemo<'m>>,
    done: Evaluated,
    /// Per rule, the clauses the memo missed.
    misses: BTreeMap<usize, Vec<usize>>,
    text: Option<Arc<String>>,
}

fn look<'m>(rules: &[Rule], memo: &'m Memo, job: &Job) -> Looked<'m> {
    let path = &*job.path;
    let read = || match &job.bytes {
        Some(b) => (**b).clone(),
        None => memo.store.get_source(job.source).unwrap_or_else(|e| panic!("store: {e}")),
    };
    let mut l = Looked { fm: None, done: Evaluated::default(), misses: BTreeMap::new(), text: None };
    if !path.ends_with(".py") {
        // A file the facts do not cover (`.rs`): its regex clauses read the text, unmemoised (as in
        // `facts-a repo`).
        let Ok(text) = String::from_utf8(read()) else {
            l.done.file_failures.push(Failure { check: String::new(), clause: String::new(), path: path.to_owned(), message: "file could not be read as UTF-8".into() });
            return l;
        };
        let mut out = eval::Output::default();
        eval::eval_text_file(rules, path, &text, &mut out);
        let mut by: BTreeMap<(usize, usize), Vec<Finding>> = BTreeMap::new();
        for f in out.findings {
            by.entry(clause_key(rules, &f.check, &f.clause)).or_default().push(f);
        }
        l.done.evaluated = by.len() as u32;
        l.done.reported = by.into_iter().map(|(clause, findings)| Reported { clause, findings, failures: Vec::new() }).collect();
        return l;
    }
    let scope = repo::in_scope(rules, path);
    if scope.is_empty() {
        return l;
    }
    let fm = memo.file_at(job.source, path);
    for &(ri, ci) in &scope {
        let (rule, clause) = (&rules[ri], &rules[ri].clauses[ci]);
        match fm.get_reported(&rule.digests[ci], &rule.check, &clause.id, path) {
            Some((findings, failures)) => {
                l.done.reported.push(Reported { clause: (ri, ci), findings, failures });
                l.done.memoised += 1;
            }
            None => l.misses.entry(ri).or_default().push(ci),
        }
    }
    l.fm = Some(fm);
    if l.misses.is_empty() {
        return l;
    }
    match String::from_utf8(read()) {
        Ok(text) => l.text = Some(Arc::new(text)),
        Err(_) => {
            l.done.file_failures.push(Failure { check: String::new(), clause: String::new(), path: path.to_owned(), message: "file is not UTF-8".into() });
            l.misses.clear();
        }
    }
    l
}

/// A task's outcome: its clauses' results, or `Err` when the pair's file did not parse.
type Outcome = Result<Vec<(Reported, Encoded)>, ()>;

/// Evaluates `jobs` on `pool`'s workers (PROTOCOL.md § Streaming): results by job, in `jobs` order.
pub fn evaluate(rules: &[Rule], memo: &Memo, jobs: &[Job], pool: &rayon::ThreadPool, progress: &mut dyn FnMut(Progress)) -> (Vec<Evaluated>, Shape) {
    let threads = pool.current_num_threads();
    // One pair (an edit) is looked up on the caller's thread: no hop to a worker.
    let mut looked: Vec<Looked> = if jobs.len() == 1 { vec![look(rules, memo, &jobs[0])] } else { pool.install(|| jobs.par_iter().map(|job| look(rules, memo, job)).collect()) };
    // Tasks, the largest texts first. With at least two pairs per worker, a task is one pair's
    // missed clauses (one facts build per pair). With fewer, a task is one rule's missed clauses on
    // one pair, in runs of at most `run` clauses so that the queue has about four tasks per worker:
    // a large file's clauses spread over the workers, each building its facts.
    let mut order: Vec<usize> = (0..jobs.len()).filter(|&j| !looked[j].misses.is_empty()).collect();
    order.sort_by_key(|&j| std::cmp::Reverse(looked[j].text.as_ref().map_or(0, |t| t.len())));
    let missed: usize = looked.iter().flat_map(|l| l.misses.values()).map(Vec::len).sum();
    let run = missed.div_ceil(4 * threads).max(1);
    let mut tasks: Vec<(usize, Vec<(usize, Vec<usize>)>)> = Vec::new();
    for &j in &order {
        let rules_missed = looked[j].misses.iter().map(|(ri, cis)| (*ri, cis.clone()));
        if order.len() >= 2 * threads {
            tasks.push((j, rules_missed.collect()));
        } else {
            tasks.extend(rules_missed.flat_map(|(ri, cis)| cis.chunks(run).map(|c| (j, vec![(ri, c.to_vec())])).collect::<Vec<_>>()));
        }
    }
    let mut served = Vec::new();
    for (j, l) in looked.iter_mut().enumerate() {
        served.extend(l.done.reported.drain(..).map(|r| (j, r)));
    }
    progress(Progress::Served { results: &served, evaluating: !tasks.is_empty() });
    for (j, r) in served {
        looked[j].done.reported.push(r);
    }
    let workers = threads.min(tasks.len());
    let mut shape = Shape { tasks: tasks.len(), workers, scratch_peak: 0 };
    if tasks.is_empty() {
        return (looked.into_iter().map(|l| l.done).collect(), shape);
    }
    let texts: Vec<Option<Arc<String>>> = looked.iter().map(|l| l.text.clone()).collect();
    // Per pair, the rule runs not yet reported (a worker reports each rule run of a task).
    let mut remaining = vec![0usize; jobs.len()];
    for (j, runs) in &tasks {
        remaining[*j] += runs.len();
    }
    let mut failed = vec![false; jobs.len()];
    let next = AtomicUsize::new(0);
    let peak = AtomicU64::new(0);
    let (tx, rx) = mpsc::channel::<(usize, usize, Outcome)>();
    pool.in_place_scope(|s| {
        for _ in 0..workers {
            let tx = tx.clone();
            let (tasks, texts, next, peak) = (&tasks, &texts, &next, &peak);
            s.spawn(move |_| {
                repo::thread_peak_reset();
                // The facts of the pair this worker's last task read (tasks are queued pair by
                // pair, so each worker builds a pair's facts at most once).
                let mut built: Option<(usize, Result<facts::Facts, ()>)> = None;
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some((j, runs)) = tasks.get(i) else { break };
                    let (path, text) = (&*jobs[*j].path, texts[*j].as_ref().expect("a pair with misses has its text"));
                    if built.as_ref().is_none_or(|b| b.0 != *j) {
                        // The previous pair's facts go before the next pair's are built.
                        drop(built.take());
                        let f = match facts::build(path, text.clone()) {
                            Ok(facts::Built::Facts(f)) => Ok(f),
                            Ok(facts::Built::Unparsed) | Err(_) => Err(()),
                        };
                        built = Some((*j, f));
                    }
                    let Ok(f) = &built.as_ref().expect("built above").1 else {
                        if tx.send((*j, runs.len(), Err(()))).is_err() {
                            break;
                        }
                        continue;
                    };
                    for (ri, cis) in runs {
                        let results = eval_rule(rules, *ri, cis, f, path, text)
                            .into_iter()
                            .map(|((ri, ci), e)| {
                                let encoded = memo.encode(ri, ci, &e, path);
                                (Reported { clause: (ri, ci), findings: e.findings, failures: e.failures }, encoded)
                            })
                            .collect();
                        if tx.send((*j, 1, Ok(results))).is_err() {
                            break;
                        }
                    }
                }
                drop(built);
                peak.fetch_max(repo::thread_peak(), Ordering::Relaxed);
            });
        }
        drop(tx);
        while let Ok(first) = rx.recv() {
            let mut batch = vec![first];
            batch.extend(rx.try_iter());
            let mut fresh: Vec<(usize, Reported)> = Vec::new();
            let mut finished = Vec::new();
            for (j, covered, outcome) in batch {
                match outcome {
                    Ok(results) => {
                        for (r, encoded) in results {
                            let (ri, ci) = r.clause;
                            looked[j].fm.as_mut().expect("a .py pair has its memo").put_encoded(ri, ci, &rules[ri].digests[ci], encoded);
                            fresh.push((j, r));
                        }
                    }
                    Err(()) => failed[j] = true,
                }
                remaining[j] -= covered;
                if remaining[j] == 0 {
                    finished.push(j);
                }
            }
            if !fresh.is_empty() {
                progress(Progress::Evaluated(&fresh));
            }
            for (j, r) in fresh {
                looked[j].done.evaluated += 1;
                looked[j].done.reported.push(r);
            }
            for j in finished {
                let l = &mut looked[j];
                let fm = l.fm.take().expect("a .py pair has its memo");
                if failed[j] {
                    // The file did not parse: every worker's build of it failed, so no clause of
                    // it was evaluated; nothing is stored.
                    l.done.reported.truncate(l.done.memoised as usize);
                    l.done.evaluated = 0;
                    l.done.file_failures.push(Failure { check: String::new(), clause: String::new(), path: jobs[j].path.to_string(), message: "file could not be parsed".into() });
                } else {
                    fm.save(texts[j].as_ref().expect("evaluated").as_bytes());
                }
            }
        }
    });
    shape.scratch_peak = peak.into_inner();
    let done = looked
        .into_iter()
        .map(|l| {
            let mut d = l.done;
            d.reported.sort_by_key(|r| r.clause);
            d
        })
        .collect();
    (done, shape)
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
