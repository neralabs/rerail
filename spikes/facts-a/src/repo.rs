//! Repository run (gates A1 repository, A2–A5) and its comparator against the suite oracle.
//!
//! `repo`: every `.py` file under the root, streamed by `--threads` workers. Each worker owns
//! its compiled rules (default configuration, production scopes, exactly as `replay.ts` resolves
//! a `production_scope` suite event) and, per file, reads, parses, builds facts, evaluates and
//! drops them. Workers return findings, timings and the repository-partition rows; the main
//! thread merges them and runs the repository partitions and budgets once (`eval::finish`).
use crate::{BATCH6, arg, call_config, eval, facts, multiset_minus, policy_doc};
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
    acc: eval::SendAcc,
    files: Vec<FileStat>,
    compile_ns: u64,
}

/// The suite's call configuration for every batch-6 check: `production_scope`, no
/// `oracle_config` (verified against the oracle by `repo-compare`).
fn compile_batch6(policies: &str) -> Vec<eval::Rule> {
    let event = json!({"production_scope": true});
    BATCH6
        .iter()
        .map(|check| {
            let doc = policy_doc(policies, check);
            let (config, scope) = call_config(&doc, &event);
            eval::Rule::compile(&doc, &config, scope.as_ref(), &facts::GRAMMAR).unwrap_or_else(|e| panic!("compile {check}: {e}"))
        })
        .collect()
}

/// Relative paths of every `.py` file under `root`, sorted; directories are entered, symlinked
/// directories are not (`os.walk` defaults).
fn walk(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
            let entry = entry.unwrap();
            let ty = entry.file_type().unwrap();
            let path = entry.path();
            if ty.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|x| x == "py") {
                out.push(path.strip_prefix(root).unwrap().to_string_lossy().into_owned());
            }
        }
    }
    out.sort();
    out
}

fn worker(paths: &[String], root: &Path, policies: &str, next: &AtomicUsize) -> Worker {
    let t0 = Instant::now();
    let rules = compile_batch6(policies);
    let compile_ns = t0.elapsed().as_nanos() as u64;
    let mut out = eval::Output::default();
    let mut repo = eval::RepoAcc::default();
    let mut budget = BTreeMap::new();
    let mut files = Vec::new();
    loop {
        let index = next.fetch_add(1, Ordering::Relaxed);
        let Some(path) = paths.get(index) else { break };
        let t_read = Instant::now();
        let bytes = std::fs::read(root.join(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
        let read_ns = t_read.elapsed().as_nanos() as u64;
        let source_bytes = bytes.len() as u32;
        let mut stat = FileStat { index: index as u32, source_bytes, nodes: 0, fact_bytes: 0, read_ns, parse_ns: 0, extract_ns: 0, eval_ns: 0 };
        let source = match String::from_utf8(bytes) {
            Ok(s) => Arc::new(s),
            Err(_) => {
                out.failures.push(eval::Failure { check: String::new(), clause: String::new(), path: path.clone(), message: "file is not UTF-8".into() });
                files.push(stat);
                continue;
            }
        };
        let mut ns = facts::BuildNs::default();
        match facts::build_timed(path, source.clone(), &mut ns) {
            Ok(facts::Built::Facts(f)) => {
                stat.nodes = f.len() as u32;
                stat.fact_bytes = f.bytes() as u64;
                let t_eval = Instant::now();
                let ctx = eval::FileCtx { f: &f, path, source: &source };
                eval::eval_file(&rules, &ctx, &mut out, &mut repo, &mut budget);
                stat.eval_ns = t_eval.elapsed().as_nanos() as u64;
            }
            Ok(facts::Built::Unparsed) | Err(_) => {
                out.failures.push(eval::Failure { check: String::new(), clause: String::new(), path: path.clone(), message: "file could not be parsed".into() });
            }
        }
        stat.parse_ns = ns.parse;
        stat.extract_ns = ns.extract;
        files.push(stat);
    }
    Worker { out, acc: repo.detach(budget), files, compile_ns }
}

/// Nearest-rank percentile of sorted values.
fn pct(sorted: &[u64], q: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = ((q * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    sorted[rank - 1]
}

fn summary(mut v: Vec<u64>) -> Json {
    v.sort_unstable();
    let sum: u64 = v.iter().sum();
    let n = v.len().max(1) as f64;
    json!({"n": v.len(), "sum": sum, "mean": sum as f64 / n, "p50": pct(&v, 0.50), "p95": pct(&v, 0.95), "p99": pct(&v, 0.99), "max": v.last().copied().unwrap_or(0)})
}

pub fn repo(args: &[String]) {
    let home = std::env::var("HOME").unwrap();
    let root = arg(args, "--root").unwrap_or(format!("{home}/rerail-bench/nera-data-layer-2b1504ad"));
    let root = Path::new(&root);
    let threads: usize = arg(args, "--threads").map_or(4, |t| t.parse().unwrap());
    let policies = arg(args, "--policies").unwrap_or("packs/data-layer/candidates/rules".into());
    let out_dir = arg(args, "--out-dir").expect("--out-dir");
    let t_all = Instant::now();
    let paths = walk(root);
    let walk_ns = t_all.elapsed().as_nanos() as u64;
    let next = AtomicUsize::new(0);
    let t_workers = Instant::now();
    let workers: Vec<Worker> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads).map(|_| s.spawn(|| worker(&paths, root, &policies, &next))).collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let workers_wall_ns = t_workers.elapsed().as_nanos() as u64;
    // Merge on this thread, then the repository partitions and budgets once.
    let t_merge = Instant::now();
    let rules = compile_batch6(&policies);
    let mut out = eval::Output::default();
    let mut parts = Vec::new();
    let mut files = Vec::new();
    let mut compile_ns = Vec::new();
    for w in workers {
        out.merge(w.out);
        parts.push(w.acc);
        files.extend(w.files);
        compile_ns.push(w.compile_ns);
    }
    let (acc, budget) = eval::attach(parts);
    let merge_ns = t_merge.elapsed().as_nanos() as u64;
    let t_finish = Instant::now();
    let per_file_ns: BTreeMap<(usize, usize), u64> = out.clause_ns.iter().map(|(k, v)| (*k, v.1)).collect();
    eval::finish(&rules, acc, budget, &mut out);
    let finish_ns = t_finish.elapsed().as_nanos() as u64;
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
            clauses.push(json!({"clause": clause.id, "calls": calls, "ns": ns, "per_file_ns": file_ns, "repository_ns": ns - file_ns, "per_file_samples": summary(samples)}));
        }
        rule_rows.push(json!({"check": rule.check, "ns": rule_ns, "clauses": clauses}));
    }
    let findings: Vec<Json> = out.findings.iter().map(|f| json!([f.check, f.path, f.line, f.code])).collect();
    let failures: Vec<Json> = out.failures.iter().map(|f| json!({"check": f.check, "clause": f.clause, "path": f.path, "message": f.message})).collect();
    let mut failure_counts: BTreeMap<String, u64> = BTreeMap::new();
    for f in &out.failures {
        *failure_counts.entry(format!("{}/{}", f.check, f.clause)).or_default() += 1;
    }
    let sum = |g: fn(&FileStat) -> u64| files.iter().map(g).sum::<u64>();
    let result = json!({
        "root": root,
        "threads": threads,
        "files": paths.len(),
        "parsed_files": parsed.len(),
        "wall_ns": wall_ns,
        "phases_ns": {
            "walk": walk_ns,
            "workers_wall": workers_wall_ns,
            "worker_compile": compile_ns,
            "merge_and_compile": merge_ns,
            "finish": finish_ns,
        },
        "thread_ns": {
            "read": sum(|f| f.read_ns),
            "parse": sum(|f| f.parse_ns),
            "extract": sum(|f| f.extract_ns),
            "eval": sum(|f| f.eval_ns),
            "input_rows": out.input_ns,
        },
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
        "failures": failures,
    });
    std::fs::write(format!("{out_dir}/results.json"), serde_json::to_string_pretty(&result).unwrap()).unwrap();
    let mut brief = result.clone();
    let o = brief.as_object_mut().unwrap();
    o.remove("findings");
    o.remove("failures");
    o.remove("rules");
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
