//! Repository run (gates A1 repository, A2–A5) and its comparator against the suite oracle.
//!
//! `repo`: every `.py` file under the root, streamed by `--threads` workers. Each worker owns
//! its compiled rules (default configuration, production scopes, exactly as `replay.ts` resolves
//! a `production_scope` suite event) and, per file, reads, parses, builds facts, evaluates and
//! drops them. Workers return findings, timings and the repository-partition rows; the main
//! thread merges them and runs the repository partitions and budgets once (`eval::finish`).
use crate::{BATCH6, arg, call_config, caught, eval, facts, memo, multiset_minus, policy_doc};
use serde_json::{Map, Value as Json, json};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// One file's costs. Nanoseconds are wall time on its worker thread.
struct FileStat {
    index: u32,
    source_bytes: u32,
    nodes: u32,
    fact_bytes: u64,
    read_ns: u64,
    parse_ns: u64,
    extract_ns: u64,
    eval_ns: u64,
}

struct Worker {
    out: eval::Output,
    budget: eval::BudgetRows,
    spill_bytes: u64,
    files: Vec<FileStat>,
    /// Per clause: (evaluations served by the memo, evaluations run).
    tally: memo::Tally,
    /// Files every in-scope clause of which the memo served (not parsed).
    memo_files: u64,
    /// `--file-counts`: one JSON row per (file, evaluated clause), `file_count_row`.
    file_rows: Vec<(String, String)>,
}

/// The inputs a clause reads.
fn clause_inputs(c: &eval::Clause) -> Vec<usize> {
    match &c.kind {
        eval::Kind::ForbidAst { input } | eval::Kind::Budget { input, .. } => vec![*input],
        eval::Kind::Relational(p) => p.inputs.iter().map(|(_, i)| *i).collect(),
        eval::Kind::ForbidRegex(_) => Vec::new(),
    }
}

/// One `--file-counts` row (docs/facts/PHASE-3.md G3b.2): the content's size (`nodes`, `depth`),
/// the clause and its class (and fallback class), its counts on the file (`crate::counts`), and
/// the matcher's physical work on the inputs it reads (`pred_tests`, shared by the rule's
/// clauses: each reader sees it).
fn file_count_row(path: &str, nodes: u64, depth: u64, rule: &eval::Rule, c: &eval::Clause, counts: &crate::counts::Counts, pred_tests: u64) -> String {
    json!({"path": path, "nodes": nodes, "depth": depth, "clause": format!("{}/{}", rule.check, c.id), "class": c.class.name(), "fallback": c.fallback.name(), "counts": counts, "pred_tests": pred_tests}).to_string()
}

/// The worker's repository sink: the spill, and while a file's results are being memoised, a
/// copy of each repository clause's rows.
struct Sink {
    spill: crate::spill::Spill,
    capture: Option<Vec<((usize, usize), crate::spill::FileRows)>>,
}

impl crate::spill::RepoSink for Sink {
    fn push(&mut self, clause: (usize, usize), rows: crate::spill::FileRows) {
        if let Some(c) = self.capture.as_mut() {
            c.push((clause, crate::spill::FileRows { path: rows.path.clone(), tables: rows.tables.clone(), anchors: rows.anchors.clone() }));
        }
        self.spill.push(clause, rows);
    }
}

/// The suite's call configuration for each selected check (default: batch 6): `production_scope`,
/// no `oracle_config` (verified against the oracle by `repo-compare`). A policy that does not compile
/// is refused by name: it becomes one failure of its check (`compile: …`) and is not evaluated.
/// Each clause gets its class overlay entry (`physplan::overlay`, beside the rule files).
pub(crate) fn compile_checks(policies: &str, checks: &[String]) -> (Vec<eval::Rule>, Vec<eval::Failure>) {
    let event = json!({"production_scope": true});
    let overlay = crate::physplan::overlay(policies);
    let (mut rules, mut refused) = (Vec::new(), Vec::new());
    for check in checks {
        let doc = policy_doc(policies, check);
        let (config, scope) = call_config(&doc, &event);
        match caught(|| eval::Rule::compile(&doc, &config, scope.as_ref(), &facts::GRAMMAR)).unwrap_or_else(|p| Err(format!("compiler panic: {p}"))) {
            Ok(mut r) => {
                for c in &mut r.clauses {
                    c.declared = overlay.get(&format!("{}/{}", r.check, c.id)).cloned();
                }
                rules.push(r)
            }
            Err(e) => refused.push(eval::Failure { check: check.clone(), clause: String::new(), path: String::new(), message: format!("compile: {e}") }),
        }
        if std::env::var_os("FACTS_MEMTRACE").is_some() {
            let rx = *crate::rx::TRACE.lock();
            eprintln!("compiled {check}: rss {} MB, heap {} MB; regexes {} holding {} MB", cur_rss() >> 20, heap_in_use() >> 20, rx.0, rx.1 >> 20);
        }
    }
    (rules, refused)
}

/// `--checks c1,c2,…` (snake-case check names), default batch 6.
pub fn selected_checks(args: &[String]) -> Vec<String> {
    arg(args, "--checks").map_or_else(|| BATCH6.iter().map(|c| c.to_string()).collect(), |s| s.split(',').map(str::to_owned).collect())
}

/// Relative paths of every `.py` file under `root`, sorted; directories are entered, symlinked
/// directories are not (`os.walk` defaults).
pub fn walk(root: &Path) -> Vec<String> {
    walk_ext(root, "py")
}

/// As `walk`, for files with extension `ext`.
fn walk_ext(root: &Path, ext: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
            let entry = entry.unwrap();
            let ty = entry.file_type().unwrap();
            let path = entry.path();
            if ty.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|x| x == ext) {
                out.push(path.strip_prefix(root).unwrap().to_string_lossy().into_owned());
            }
        }
    }
    out.sort();
    out
}

/// The clauses `rules` evaluate on `path`: every clause of every rule whose scope matches it.
fn in_scope(rules: &[eval::Rule], path: &str) -> Vec<(usize, usize)> {
    rules.iter().enumerate().filter(|(_, r)| r.scope.matches(path)).flat_map(|(ri, r)| (0..r.clauses.len()).map(move |ci| (ri, ci))).collect()
}

#[allow(clippy::too_many_arguments)]
fn worker(rules: &[eval::Rule], paths: &[String], root: &Path, next: &AtomicUsize, spill_dir: &Path, wi: usize, memo_dir: Option<&Path>, file_counts: bool) -> Worker {
    let mut out = eval::Output::default();
    let mut repo = Sink { spill: crate::spill::Spill::new(spill_dir, wi), capture: None };
    let mut budget: eval::BudgetRows = BTreeMap::new();
    let mut files = Vec::new();
    let mut tally = memo::Tally::new();
    let mut memo_files = 0u64;
    let mut file_rows = Vec::new();
    loop {
        let index = next.fetch_add(1, Ordering::Relaxed);
        let Some(path) = paths.get(index) else { break };
        let t_read = Instant::now();
        let bytes = std::fs::read(root.join(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
        let read_ns = t_read.elapsed().as_nanos() as u64;
        let source_bytes = bytes.len() as u32;
        let mut stat = FileStat { index: index as u32, source_bytes, nodes: 0, fact_bytes: 0, read_ns, parse_ns: 0, extract_ns: 0, eval_ns: 0 };
        // The memo: the in-scope clauses it holds for this (content, path) are served from it.
        let mut fm = memo_dir.map(|d| memo::FileMemo::load(d, blake3::hash(&bytes).as_bytes(), path));
        let scope = in_scope(rules, path);
        let hits: Vec<(usize, usize)> = fm.as_ref().map_or_else(Vec::new, |m| scope.iter().copied().filter(|&(ri, ci)| m.has(&rules[ri].digests[ci])).collect());
        if let Some(m) = &fm {
            let rc: std::rc::Rc<str> = std::rc::Rc::from(path.as_str());
            for &(ri, ci) in &hits {
                let (rule, clause) = (&rules[ri], &rules[ri].clauses[ci]);
                let e = m.get(&rule.digests[ci], &rule.check, &clause.id, &rc).expect("memo entry");
                out.findings.extend(e.findings);
                out.failures.extend(e.failures);
                *out.clause_counts.entry((ri, ci)).or_default() += &e.counts;
                if let Some(rows) = e.rows {
                    crate::spill::RepoSink::push(&mut repo.spill, (ri, ci), rows);
                }
                budget.entry((ri, ci)).or_default().extend(e.budget);
                tally.entry((ri, ci)).or_default().0 += 1;
            }
            if !scope.is_empty() && hits.len() == scope.len() {
                memo_files += 1;
                files.push(stat);
                continue;
            }
        }
        let source = match String::from_utf8(bytes) {
            Ok(s) => Arc::new(s),
            Err(_) => {
                out.failures.push(eval::Failure { check: String::new(), clause: String::new(), path: path.clone(), message: "file is not UTF-8".into() });
                files.push(stat);
                continue;
            }
        };
        let misses: Vec<(usize, usize)> = scope.iter().copied().filter(|k| !hits.contains(k)).collect();
        for k in &misses {
            tally.entry(*k).or_default().1 += 1;
        }
        out.skip = hits.into_iter().collect();
        let (f0, x0) = (out.findings.len(), out.failures.len());
        let b0: BTreeMap<(usize, usize), usize> = misses.iter().map(|k| (*k, budget.get(k).map_or(0, Vec::len))).collect();
        if fm.is_some() || file_counts {
            out.file_counts = Some(Vec::new());
        }
        if fm.is_some() {
            repo.capture = Some(Vec::new());
        }
        // The matcher's physical work so far, per input (`--file-counts` reports this file's).
        let tests0: BTreeMap<(usize, usize), u64> = if file_counts { out.input_counts.iter().map(|(k, c)| (*k, c.pred_tests)).collect() } else { BTreeMap::new() };
        let mut ns = facts::BuildNs::default();
        match facts::build_timed(path, source.clone(), &mut ns) {
            Ok(facts::Built::Facts(f)) => {
                stat.nodes = f.len() as u32;
                stat.fact_bytes = f.bytes() as u64;
                let t_eval = Instant::now();
                let ctx = eval::FileCtx { f: &f, path, source: &source };
                eval::eval_file(rules, &ctx, &mut out, &mut repo, &mut budget);
                stat.eval_ns = t_eval.elapsed().as_nanos() as u64;
                let counts = out.file_counts.take().unwrap_or_default();
                if file_counts {
                    let depth = u64::from(f.depth());
                    let tests = |ri: usize, ii: usize| out.input_counts.get(&(ri, ii)).map_or(0, |c| c.pred_tests) - tests0.get(&(ri, ii)).copied().unwrap_or(0);
                    for ((ri, ci), c) in &counts {
                        let (rule, clause) = (&rules[*ri], &rules[*ri].clauses[*ci]);
                        let pred_tests = clause_inputs(clause).into_iter().map(|ii| tests(*ri, ii)).sum();
                        file_rows.push((path.clone(), file_count_row(path, f.len() as u64, depth, rule, clause, c, pred_tests)));
                    }
                }
                // Each evaluated clause's results become its memo entry.
                if let Some(m) = fm.as_mut() {
                    let mut rows = repo.capture.take().unwrap_or_default();
                    for &(ri, ci) in &misses {
                        let (rule, clause) = (&rules[ri], &rules[ri].clauses[ci]);
                        let mine = |check: &str, cl: &str| check == rule.check && cl == clause.id;
                        let e = memo::Entry {
                            findings: out.findings[f0..].iter().filter(|x| mine(&x.check, &x.clause)).cloned().collect(),
                            failures: out.failures[x0..].iter().filter(|x| mine(&x.check, &x.clause)).cloned().collect(),
                            counts: counts.iter().filter(|(k, _)| *k == (ri, ci)).fold(Default::default(), |mut a, (_, c)| {
                                a += c;
                                a
                            }),
                            rows: rows.iter().position(|(k, _)| *k == (ri, ci)).map(|i| rows.swap_remove(i).1),
                            budget: budget.get(&(ri, ci)).map_or_else(Vec::new, |v| {
                                v[b0[&(ri, ci)]..].iter().map(|b| eval::BudgetRow { path: b.path.clone(), key: b.key.clone(), line: b.line, values: b.values.clone() }).collect()
                            }),
                        };
                        m.put(&rule.digests[ci], &e);
                    }
                }
            }
            Ok(facts::Built::Unparsed) | Err(_) => {
                out.failures.push(eval::Failure { check: String::new(), clause: String::new(), path: path.clone(), message: "file could not be parsed".into() });
                fm = None;
            }
        }
        out.file_counts = None;
        repo.capture = None;
        out.skip.clear();
        if let Some(m) = fm {
            m.save();
        }
        stat.parse_ns = ns.parse;
        stat.extract_ns = ns.extract;
        files.push(stat);
    }
    Worker { out, budget, spill_bytes: repo.spill.finish(), files, tally, memo_files, file_rows }
}

/// Nearest-rank percentile of sorted values.
fn pct(sorted: &[u64], q: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = ((q * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    sorted[rank - 1]
}

pub fn summary(mut v: Vec<u64>) -> Json {
    v.sort_unstable();
    let sum: u64 = v.iter().sum();
    let n = v.len().max(1) as f64;
    json!({"n": v.len(), "sum": sum, "mean": sum as f64 / n, "p50": pct(&v, 0.50), "p95": pct(&v, 0.95), "p99": pct(&v, 0.99), "max": v.last().copied().unwrap_or(0)})
}

/// The process's peak resident set so far, in bytes (macOS reports `ru_maxrss` in bytes).
fn max_rss() -> u64 {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: `usage` is a valid, writable `rusage`.
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    usage.ru_maxrss as u64
}

/// The process's current resident set, in bytes (`proc_pidinfo` task info).
pub fn cur_rss() -> u64 {
    let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
    // SAFETY: `info` is a valid, writable `proc_taskinfo` of `size` bytes.
    let n = unsafe { libc::proc_pidinfo(std::process::id() as libc::c_int, libc::PROC_PIDTASKINFO, 0, &mut info as *mut _ as *mut libc::c_void, size) };
    if n == size { info.pti_resident_size } else { 0 }
}

/// Live heap bytes (jemalloc's `stats.allocated`, after an epoch advance): unlike RSS, this falls
/// when memory is freed. Costs a stats merge; for tracing and once-per-run measurements.
pub fn heap_in_use() -> u64 {
    let _ = tikv_jemalloc_ctl::epoch::advance();
    tikv_jemalloc_ctl::stats::allocated::read().unwrap_or(0) as u64
}

/// This thread's peak net allocation since `thread_peak_reset` (jemalloc `thread.peak.read`).
pub fn thread_peak() -> u64 {
    // SAFETY: `thread.peak.read` is a u64 read-only mallctl.
    unsafe { tikv_jemalloc_ctl::raw::read::<u64>(b"thread.peak.read\0") }.unwrap_or(0)
}

/// Restarts `thread_peak` from this thread's current net allocation.
pub fn thread_peak_reset() {
    // SAFETY: `thread.peak.reset` takes and returns nothing: every pointer null, length 0.
    unsafe { tikv_jemalloc_sys::mallctl(c"thread.peak.reset".as_ptr(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), 0) };
}

pub fn repo(args: &[String]) {
    let home = std::env::var("HOME").unwrap();
    let root = arg(args, "--root").unwrap_or(format!("{home}/rerail-bench/nera-data-layer-2b1504ad"));
    let root = Path::new(&root);
    let threads: usize = arg(args, "--threads").map_or(4, |t| t.parse().unwrap());
    let policies = arg(args, "--policies").unwrap_or("packs/data-layer/candidates/rules".into());
    let checks = selected_checks(args);
    let out_dir = arg(args, "--out-dir").expect("--out-dir");
    let t_all = Instant::now();
    let paths = walk(root);
    let walk_ns = t_all.elapsed().as_nanos() as u64;
    let next = AtomicUsize::new(0);
    // Repository-partition rows go to a per-run temporary directory during the file phase.
    let spill_dir = std::env::temp_dir().join(format!("facts-a-spill-{}", std::process::id()));
    std::fs::create_dir_all(&spill_dir).unwrap();
    // The compiled rules: one immutable object per process, shared by every worker and by the
    // repository phase.
    let (rss_before_compile, heap_before_compile) = (cur_rss(), heap_in_use());
    let t_compile = Instant::now();
    let (mut rules, refused) = compile_checks(&policies, &checks);
    // `--clauses a,b`: only these clauses of the selected checks (the archetype bench).
    if let Some(keep) = arg(args, "--clauses") {
        let keep: Vec<&str> = keep.split(',').collect();
        rules.iter_mut().for_each(|r| r.retain_clauses(|c| keep.contains(&c)));
    }
    let rules: Arc<[eval::Rule]> = rules.into();
    let compile_ns = t_compile.elapsed().as_nanos() as u64;
    let compiled_rss = cur_rss().saturating_sub(rss_before_compile);
    let compiled_heap = heap_in_use().saturating_sub(heap_before_compile);
    let memo_dir = memo::dir(args);
    // `--file-counts PATH`: per (file, evaluated clause) counts as JSON lines, sorted by path
    // (the exponent lane, docs/facts/PHASE-3.md G3b.2).
    let file_counts = arg(args, "--file-counts");
    let t_workers = Instant::now();
    let workers: Vec<Worker> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads).map(|wi| {
            let (rules, paths, next, spill_dir, memo_dir) = (&*rules, &paths, &next, &spill_dir, memo_dir.as_deref());
            let fc = file_counts.is_some();
            s.spawn(move || worker(rules, paths, root, next, spill_dir, wi, memo_dir, fc))
        }).collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let workers_wall_ns = t_workers.elapsed().as_nanos() as u64;
    let rss_after_workers = max_rss();
    let cur_after_workers = cur_rss();
    // Merge on this thread, then the repository partitions and budgets once.
    let t_merge = Instant::now();

    let mut out = eval::Output::default();
    out.failures.extend(refused);
    // Files the facts do not cover (`.rs`): regex clauses read their text, as in `facts-a calls`
    // (`eval::eval_text_file`); AST clauses over the Python grammar read no such file.
    for path in walk_ext(root, "rs") {
        match std::fs::read_to_string(root.join(&path)) {
            Ok(source) => eval::eval_text_file(&rules, &path, &source, &mut out),
            Err(e) => out.failures.push(eval::Failure { check: String::new(), clause: String::new(), path: path.clone(), message: format!("file could not be read as UTF-8: {e}") }),
        }
    }
    let mut budget: eval::BudgetRows = BTreeMap::new();
    let mut files = Vec::new();
    let mut spill_bytes = 0u64;
    let mut tally = memo::Tally::new();
    let mut memo_files = 0u64;
    let mut file_rows = Vec::new();
    for w in workers {
        out.merge(w.out);
        for (k, rows) in w.budget {
            budget.entry(k).or_default().extend(rows);
        }
        files.extend(w.files);
        spill_bytes += w.spill_bytes;
        for (k, (h, e)) in w.tally {
            let t = tally.entry(k).or_default();
            t.0 += h;
            t.1 += e;
        }
        memo_files += w.memo_files;
        file_rows.extend(w.file_rows);
    }
    if let Some(p) = &file_counts {
        // Stable order: by path, a file's rows in evaluation order.
        file_rows.sort_by(|a, b| a.0.cmp(&b.0));
        let text: String = file_rows.into_iter().map(|(_, row)| row + "\n").collect();
        std::fs::write(p, text).unwrap_or_else(|e| panic!("{p}: {e}"));
    }
    let merge_ns = t_merge.elapsed().as_nanos() as u64;
    let rss_after_merge = max_rss();
    let cur_after_merge = cur_rss();
    let t_finish = Instant::now();
    let per_file_ns: BTreeMap<(usize, usize), u64> = out.clause_ns.iter().map(|(k, v)| (*k, v.1)).collect();
    eval::finish(&rules, |k| crate::spill::load(&spill_dir, threads, k), budget, &mut out);
    std::fs::remove_dir_all(&spill_dir).unwrap();
    let finish_ns = t_finish.elapsed().as_nanos() as u64;
    let rss_after_finish = max_rss();
    let cur_after_finish = cur_rss();
    let wall_ns = t_all.elapsed().as_nanos() as u64;

    files.sort_by_key(|f| f.index);
    let mut tsv = String::from("path\tsource_bytes\tnodes\tfact_bytes\tread_ns\tparse_ns\textract_ns\teval_ns\n");
    for f in &files {
        tsv.push_str(&format!("{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n", paths[f.index as usize], f.source_bytes, f.nodes, f.fact_bytes, f.read_ns, f.parse_ns, f.extract_ns, f.eval_ns));
    }
    std::fs::write(format!("{out_dir}/per-file.tsv"), tsv).unwrap();
    let parsed: Vec<&FileStat> = files.iter().filter(|f| f.nodes > 0).collect();
    let mut samples_by_clause: BTreeMap<(u16, u16), Vec<u64>> = BTreeMap::new();
    for (k, ns) in &out.clause_samples {
        samples_by_clause.entry(*k).or_default().push(*ns);
    }
    let mut rule_rows = Vec::new();
    for (ri, rule) in rules.iter().enumerate() {
        let mut clauses = Vec::new();
        let mut rule_ns = 0u64;
        for (ci, clause) in rule.clauses.iter().enumerate() {
            let (calls, ns) = out.clause_ns.get(&(ri, ci)).copied().unwrap_or((0, 0));
            rule_ns += ns;
            let samples = samples_by_clause.remove(&(ri as u16, ci as u16)).unwrap_or_default();
            let file_ns = per_file_ns.get(&(ri, ci)).copied().unwrap_or(0);
            let counts = out.clause_counts.get(&(ri, ci)).cloned().unwrap_or_default();
            // Per-file evaluations served by the memo and run (`crate::memo`).
            let (memoised, evaluated) = tally.get(&(ri, ci)).copied().unwrap_or((0, 0));
            clauses.push(json!({"clause": clause.id, "calls": calls, "ns": ns, "per_file_ns": file_ns, "repository_ns": ns - file_ns, "per_file_samples": summary(samples), "counts": counts,
                "digest": memo::hex(&rule.digests[ci]), "memo": {"memoised": memoised, "evaluated": evaluated}}));
        }
        rule_rows.push(json!({"check": rule.check, "ns": rule_ns, "clauses": clauses}));
    }
    let findings: Vec<Json> = out.findings.iter().map(|f| json!([f.check, f.path, f.line, f.code])).collect();
    // The same findings with their clause (the loop's sub200 reference compares per clause).
    let clause_findings: Vec<Json> = out.findings.iter().map(|f| json!([f.check, f.clause, f.path, f.line, f.code])).collect();
    let failures: Vec<Json> = out.failures.iter().map(|f| json!({"check": f.check, "clause": f.clause, "path": f.path, "message": f.message})).collect();
    let mut failure_counts: BTreeMap<String, u64> = BTreeMap::new();
    for f in &out.failures {
        *failure_counts.entry(format!("{}/{}", f.check, f.clause)).or_default() += 1;
    }
    let sum = |g: fn(&FileStat) -> u64| files.iter().map(g).sum::<u64>();
    let mut stage_agg: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
    for ((_, _, k), (n, ns)) in &out.stage_ns {
        let e = stage_agg.entry(k.as_str()).or_default();
        e.0 += n;
        e.1 += ns;
    }
    let result = json!({
        "root": root,
        "threads": threads,
        "files": paths.len(),
        "parsed_files": parsed.len(),
        // The digest memo (`crate::memo`): per-file clause evaluations served by it and run, and
        // the files it served whole (not parsed); `dir` null with `--no-memo` or a mutant.
        "memo": {
            "dir": memo_dir,
            "lookups": tally.values().map(|t| t.0 + t.1).sum::<u64>(),
            "hits": tally.values().map(|t| t.0).sum::<u64>(),
            "evaluated": tally.values().map(|t| t.1).sum::<u64>(),
            "files_served": memo_files,
        },
        "wall_ns": wall_ns,
        "phases_ns": {
            "walk": walk_ns,
            "workers_wall": workers_wall_ns,
            "compile": compile_ns,
            "merge": merge_ns,
            "finish": finish_ns,
        },
        "spill_bytes": spill_bytes,
        // What the compiled rules added: resident bytes, and live heap bytes (the object's size).
        "compiled_rules_rss_bytes": compiled_rss,
        "compiled_rules_heap_bytes": compiled_heap,
        "max_rss_bytes": {
            "after_workers": rss_after_workers,
            "after_merge": rss_after_merge,
            "after_finish": rss_after_finish,
        },
        "cur_rss_bytes": {
            "after_workers": cur_after_workers,
            "after_merge": cur_after_merge,
            "after_finish": cur_after_finish,
        },
        "sql_stage_ns": stage_agg.into_iter().map(|(k, (n, ns))| (k.to_owned(), json!([n, ns]))).collect::<serde_json::Map<String, Json>>(),
        "clause_stage_ns": out.stage_ns.iter().map(|((ri, ci, k), (n, ns))| json!({"check": rules[*ri].check, "clause": rules[*ri].clauses[*ci].id, "stage": k, "n": n, "ns": ns})).collect::<Vec<Json>>(),
        // Work counts (`crate::counts`): the run's totals, per stage of each clause, per input.
        "counts": out.clause_counts.values().fold(crate::counts::Counts::default(), |mut a, c| {
            a += c;
            a
        }),
        "clause_stage_counts": out.stage_counts.iter().map(|((ri, ci, k), s)| json!({"check": rules[*ri].check, "clause": rules[*ri].clauses[*ci].id, "stage": k, "counts": s})).collect::<Vec<Json>>(),
        "input_counts": out.input_counts.iter().map(|((ri, ii), c)| json!({"check": rules[*ri].check, "input": ii, "counts": c})).collect::<Vec<Json>>(),
        "thread_ns": {
            "read": sum(|f| f.read_ns),
            "parse": sum(|f| f.parse_ns),
            "extract": sum(|f| f.extract_ns),
            "eval": sum(|f| f.eval_ns),
            "input_rows": out.input_ns.values().map(|v| v.1).sum::<u64>(),
        },
        "input_ns": out.input_ns.iter().map(|((ri, ii), (n, ns))| {
            let rule = &rules[*ri];
            let names: Vec<String> = rule.clauses.iter().filter_map(|c| match &c.kind {
                eval::Kind::Relational(p) => p.inputs.iter().find(|(_, idx)| idx == ii).map(|(name, _)| format!("{}/{name}", c.id)),
                eval::Kind::ForbidAst { input } | eval::Kind::Budget { input, .. } if input == ii => Some(c.id.clone()),
                _ => None,
            }).collect();
            json!({"check": rule.check, "input": ii, "used_by": names, "builds": n, "ns": ns})
        }).collect::<Vec<Json>>(),
        "a3_clause_file_ns": summary(out.clause_samples.iter().map(|(_, ns)| *ns).collect()),
        "a4": {
            "parse_ns": summary(parsed.iter().map(|f| f.parse_ns).collect()),
            "extract_ns": summary(parsed.iter().map(|f| f.extract_ns).collect()),
            "parse_plus_extract_ns": summary(parsed.iter().map(|f| f.parse_ns + f.extract_ns).collect()),
            "fact_bytes_in_memory": summary(parsed.iter().map(|f| f.fact_bytes).collect()),
            "source_bytes": summary(parsed.iter().map(|f| f.source_bytes as u64).collect()),
            "nodes": summary(parsed.iter().map(|f| f.nodes as u64).collect()),
        },
        "rules": rule_rows,
        "finding_count": findings.len(),
        "failure_counts": failure_counts,
        "findings": findings,
        "clause_findings": clause_findings,
        "failures": failures,
    });
    std::fs::write(format!("{out_dir}/results.json"), serde_json::to_string_pretty(&result).unwrap()).unwrap();
    let mut brief = result.clone();
    let o = brief.as_object_mut().unwrap();
    o.remove("findings");
    o.remove("clause_findings");
    o.remove("failures");
    o.remove("rules");
    o.remove("clause_stage_counts");
    o.remove("input_counts");
    o.remove("sql_stage_ns");
    o.remove("clause_stage_ns");
    o.remove("input_ns");
    println!("{}", serde_json::to_string_pretty(&brief).unwrap());
}

/// Parity of a `repo` run with the suite oracle: multisets of `[path, line, code]` per check,
/// the `replay.ts` matching rule, plus every failure. Checks the oracle's events use the
/// configuration `repo` assumed (production scope, no extra options, nothing skipped) and the
/// same file set.
pub fn compare(args: &[String]) {
    let oracle_path = arg(args, "--oracle").expect("--oracle");
    let results_path = arg(args, "--results").expect("--results");
    let out_path = arg(args, "--output").expect("--output");
    let oracle: Json = serde_json::from_str(&std::fs::read_to_string(&oracle_path).unwrap()).unwrap();
    let results: Json = serde_json::from_str(&std::fs::read_to_string(&results_path).unwrap()).unwrap();
    let key_of = |p: &str, l: u64, c: &str| format!("{p}\u{1f}{l}\u{1f}{c}");
    let mut config_issues = Vec::new();
    let mut expected: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for e in oracle["events"].as_array().unwrap() {
        let check = e["check"].as_str().unwrap().to_owned();
        if e.get("production_scope") != Some(&Json::Bool(true)) || e.get("oracle_config").is_some_and(|c| !c.is_null()) || e.get("retained_reason").is_some() || e.get("upstream_passed") == Some(&Json::Bool(false)) {
            config_issues.push(json!({"check": check, "event": {"production_scope": e.get("production_scope"), "oracle_config": e.get("oracle_config"), "retained_reason": e.get("retained_reason"), "upstream_passed": e.get("upstream_passed")}}));
        }
        let v = expected.entry(check).or_default();
        for f in e["findings"].as_array().unwrap() {
            v.push(key_of(f["path"].as_str().unwrap(), f["line"].as_u64().unwrap(), f["code"].as_str().unwrap()));
        }
    }
    let oracle_files: std::collections::BTreeSet<&str> = oracle["files"].as_array().unwrap().iter().map(|f| f["path"].as_str().unwrap()).collect();
    let root = Path::new(results["root"].as_str().unwrap());
    let walked: std::collections::BTreeSet<String> = walk(root).into_iter().collect();
    let files_missing: Vec<&&str> = oracle_files.iter().filter(|p| !walked.contains(**p)).collect();
    let files_extra: Vec<&String> = walked.iter().filter(|p| !oracle_files.contains(p.as_str())).collect();
    let mut actual: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for f in results["findings"].as_array().unwrap() {
        actual.entry(f[0].as_str().unwrap().to_owned()).or_default().push(key_of(f[1].as_str().unwrap(), f[2].as_u64().unwrap(), f[3].as_str().unwrap()));
    }
    let mut failures: BTreeMap<String, Vec<Json>> = BTreeMap::new();
    for f in results["failures"].as_array().unwrap() {
        failures.entry(f["check"].as_str().unwrap().to_owned()).or_default().push(f.clone());
    }
    let mut per_check = Map::new();
    let mut totals = (0usize, 0usize, 0usize, 0usize);
    let checks: std::collections::BTreeSet<String> = BATCH6.iter().map(|c| c.to_string()).chain(failures.keys().cloned()).collect();
    for check in checks {
        let mut exp = expected.get(&check).cloned().unwrap_or_default();
        let mut act = actual.get(&check).cloned().unwrap_or_default();
        exp.sort();
        act.sort();
        let fp = multiset_minus(&act, &exp);
        let fnn = multiset_minus(&exp, &act);
        let fails = failures.get(&check).cloned().unwrap_or_default();
        let mut by_clause: BTreeMap<String, u64> = BTreeMap::new();
        for f in &fails {
            *by_clause.entry(format!("{} ({})", f["clause"].as_str().unwrap(), f["message"].as_str().unwrap())).or_default() += 1;
        }
        totals.0 += exp.len();
        totals.1 += fp.len();
        totals.2 += fnn.len();
        totals.3 += fails.len();
        per_check.insert(
            check.clone(),
            json!({"expected": exp.len(), "actual": act.len(), "false_positives": fp.len(), "false_negatives": fnn.len(), "failures": fails.len(), "failures_by_clause": by_clause, "fp": fp, "fn": fnn}),
        );
    }
    let report = json!({
        "oracle": oracle_path,
        "results": results_path,
        "config_issues": config_issues,
        "files": {"oracle": oracle_files.len(), "walked": walked.len(), "missing": files_missing, "extra": files_extra},
        "totals": {"expected": totals.0, "false_positives": totals.1, "false_negatives": totals.2, "failures": totals.3},
        "checks": per_check,
    });
    std::fs::write(&out_path, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    let mut brief = report.clone();
    for (_, c) in brief["checks"].as_object_mut().unwrap() {
        let o = c.as_object_mut().unwrap();
        o.remove("fp");
        o.remove("fn");
    }
    println!("{}", serde_json::to_string_pretty(&brief).unwrap());
}
