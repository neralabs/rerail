//! `facts-b extract`: scan the corpus worktrees into memberships, then extract the exported facts
//! of every distinct content once (4 workers) into the shared store. Timed separately from the
//! steady state.
use crate::facts;
use crate::store::{self, Membership};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use parking_lot::Mutex;
use std::time::Instant;

pub struct Job {
    pub hash: [u8; 32],
    pub source: PathBuf,
}

pub struct Extracted {
    pub ns: u64,
    pub src_bytes: usize,
    pub fact_bytes: usize,
    pub rows: [usize; 8],
}

/// Extract, encode and write one content's facts. Returns timing and sizes.
pub fn extract_one(parser: &mut tree_sitter::Parser, store_dir: &Path, hash: &[u8; 32], src: &[u8]) -> Extracted {
    let t = Instant::now();
    let tree = facts::parse(parser, src);
    let e = facts::extract(src, &tree);
    let ns = t.elapsed().as_nanos() as u64;
    let bytes = facts::encode(hash, &e);
    let hex = facts::hex(hash);
    let p = facts::store_path(store_dir, &hex);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    let tmp = p.with_extension(format!("xfacts.tmp{}", std::process::id()));
    std::fs::write(&tmp, &bytes).unwrap();
    std::fs::rename(&tmp, &p).unwrap();
    Extracted {
        ns,
        src_bytes: src.len(),
        fact_bytes: bytes.len(),
        rows: [
            e.defs.len(),
            e.refs.len(),
            e.imports.len(),
            e.calls.len(),
            e.decorators.len(),
            e.dunder_all_names.len(),
            e.seeds.len(),
            e.has_error as usize,
        ],
    }
}

fn pct(v: &mut [u64], p: f64) -> u64 {
    v.sort_unstable();
    if v.is_empty() {
        return 0;
    }
    v[((v.len() - 1) as f64 * p).round() as usize]
}

pub fn run(corpus: &Path, run_dir: &Path, threads: usize) {
    std::fs::create_dir_all(run_dir).unwrap();
    let mdir = store::spike_dir().join("membership");
    std::fs::create_dir_all(&mdir).unwrap();
    let store_dir = store::store_dir();
    // Scan.
    let t_scan = Instant::now();
    let mut names: Vec<String> = std::fs::read_dir(corpus).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    let mut distinct: BTreeMap<[u8; 32], PathBuf> = BTreeMap::new();
    let mut total_files = 0usize;
    let mut per_wt = BTreeMap::new();
    for w in &names {
        let root = corpus.join(w);
        let mut m: Membership = Vec::new();
        for rel in store::walk_py(&root) {
            let full = root.join(&rel);
            let bytes = std::fs::read(&full).unwrap();
            let h: [u8; 32] = blake3::hash(&bytes).into();
            distinct.entry(h).or_insert(full);
            m.push((rel, h));
        }
        total_files += m.len();
        per_wt.insert(w.clone(), m.len());
        store::write_membership(&mdir.join(format!("{w}.tsv")), &m);
    }
    let scan_s = t_scan.elapsed().as_secs_f64();
    eprintln!("scan: {} worktrees, {} files, {} distinct, {:.1}s", names.len(), total_files, distinct.len(), scan_s);

    // Extract every distinct content once.
    let jobs: Vec<Job> = distinct.into_iter().map(|(hash, source)| Job { hash, source }).collect();
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Extracted>> = Mutex::new(Vec::with_capacity(jobs.len()));
    let t_ex = Instant::now();
    let cpu0 = cpu_seconds();
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                let mut parser = facts::parser();
                let mut local = Vec::new();
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= jobs.len() {
                        break;
                    }
                    let src = std::fs::read(&jobs[i].source).unwrap();
                    local.push(extract_one(&mut parser, &store_dir, &jobs[i].hash, &src));
                }
                results.lock().extend(local);
            });
        }
    });
    let wall = t_ex.elapsed().as_secs_f64();
    let cpu = cpu_seconds() - cpu0;
    let res = results.into_inner();
    let mut ns: Vec<u64> = res.iter().map(|r| r.ns).collect();
    let parse_extract_cpu: f64 = ns.iter().sum::<u64>() as f64 / 1e9;
    let src_bytes: usize = res.iter().map(|r| r.src_bytes).sum();
    let fact_bytes: usize = res.iter().map(|r| r.fact_bytes).sum();
    let mut rows = [0usize; 8];
    for r in &res {
        for i in 0..8 {
            rows[i] += r.rows[i];
        }
    }
    let mut fb: Vec<u64> = res.iter().map(|r| r.fact_bytes as u64).collect();
    let out = json!({
        "corpus": corpus.display().to_string(),
        "worktrees": names.len(),
        "files": total_files,
        "per_worktree_files": per_wt,
        "distinct_contents": res.len(),
        "scan_seconds": scan_s,
        "threads": threads,
        "extract_wall_seconds": wall,
        "extract_process_cpu_seconds": cpu,
        "parse_extract_cpu_seconds": parse_extract_cpu,
        "per_file_ms": {
            "p50": pct(&mut ns, 0.5) as f64 / 1e6,
            "p95": pct(&mut ns, 0.95) as f64 / 1e6,
            "p99": pct(&mut ns, 0.99) as f64 / 1e6,
            "max": pct(&mut ns, 1.0) as f64 / 1e6,
            "mean": parse_extract_cpu * 1e3 / res.len() as f64,
        },
        "source_bytes_total": src_bytes,
        "fact_bytes_total": fact_bytes,
        "fact_bytes_per_content": {"mean": fact_bytes as f64 / res.len() as f64, "p50": pct(&mut fb, 0.5), "p99": pct(&mut fb, 0.99), "max": pct(&mut fb, 1.0)},
        "rows_total": {"def": rows[0], "ref": rows[1], "import": rows[2], "call_edge": rows[3], "def_decorator": rows[4], "dunder_all_name": rows[5], "seed": rows[6], "syntax_error_contents": rows[7]},
    });
    std::fs::write(run_dir.join("results.json"), serde_json::to_string_pretty(&out).unwrap()).unwrap();
    println!("{}", serde_json::to_string_pretty(&out).unwrap());
}

pub fn cpu_seconds() -> f64 {
    let mut u: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
    u.ru_utime.tv_sec as f64 + u.ru_utime.tv_usec as f64 / 1e6 + u.ru_stime.tv_sec as f64 + u.ru_stime.tv_usec as f64 / 1e6
}
