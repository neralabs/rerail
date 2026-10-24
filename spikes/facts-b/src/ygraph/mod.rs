//! Y1: the reference graph as a dedicated maintained structure (exploration, throwaway).
//! `facts-b ygraph parity|replay …`; see docs/facts/reports/explore-y1-graph.md.
pub mod engine;
pub mod intern;
pub mod link;
pub mod path;
pub mod replay;
pub mod summary;
pub mod view;
pub mod world;

use std::path::PathBuf;
use world::*;

pub fn bench() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap()).join("rerail-bench/facts")
}

pub fn rule_path() -> PathBuf {
    bench().join("phase-2/s3-crossfile/exports/w3/policies/data-layer-internal-dead-code.json")
}

pub fn shared() -> Shared {
    Shared {
        it: intern::Interner::default(),
        sums: summary::Summaries::default(),
        qt: engine::QTable::default(),
        defs: DefTable::default(),
        anomalies: engine::Anomalies::default(),
        scope: Scope::load(&rule_path()),
    }
}

pub fn read_members(p: &std::path::Path) -> Vec<(String, String)> {
    std::fs::read_to_string(p)
        .unwrap_or_else(|e| panic!("{}: {e}", p.display()))
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let mut f = l.split('\t');
            (f.next().unwrap().to_owned(), f.next().unwrap().to_owned())
        })
        .collect()
}

/// Build a worktree's graph from scratch: every (path, content) member added over no layers.
pub fn scratch(sh: &mut Shared, members: &[(intern::Sym, intern::Sym)]) -> (Delta, Step) {
    let mut d = Delta::default();
    let changes: Vec<Change> = members.iter().map(|&(p, c)| Change { path: p, old: None, new: Some(c) }).collect();
    let step = apply(sh, &Below::default(), &mut d, &changes);
    (d, step)
}

pub fn run_big<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new().stack_size(4 << 30).spawn(f).unwrap().join().unwrap()
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

pub fn main(args: Vec<String>) {
    match args.get(2).map(String::as_str) {
        Some("parity") => run_big(move || parity(&args)),
        Some("replay") => run_big(move || replay::run(&args)),
        _ => {
            eprintln!("usage: facts-b ygraph parity|replay [...]");
            std::process::exit(2);
        }
    }
}

/// `ygraph parity`: the base builder on the 2b1504adc corpus against native's reference graph.
fn parity(args: &[String]) {
    let b = bench().join("phase-2/s3-crossfile");
    let members = PathBuf::from(arg(args, "--members").unwrap_or_else(|| b.join("exports/oracle-b6-jobs.tsv").display().to_string()));
    let exports = PathBuf::from(arg(args, "--exports").unwrap_or_else(|| b.join("exports/w6/oracle-dead-code.jsonl").display().to_string()));
    let native = PathBuf::from(arg(args, "--native").unwrap_or_else(|| b.join("linker/native-ground-truth/reference_edges.tsv").display().to_string()));
    let out_dir = PathBuf::from(arg(args, "--out").expect("--out DIR"));
    std::fs::create_dir_all(&out_dir).unwrap();
    let mut sh = shared();
    let ms = read_members(&members);
    let want: rustc_hash::FxHashSet<(String, String)> = ms.iter().cloned().collect();
    let t = std::time::Instant::now();
    let n = summary::load(&exports, &mut sh.it, &mut sh.sums, Some(&|p: &str, h: &str| want.contains(&(p.to_owned(), h.to_owned()))));
    let load_s = t.elapsed().as_secs_f64();
    let fp_loaded = replay::mem_json();
    let mem: Vec<(u32, u32)> = ms.iter().map(|(p, h)| (sh.it.intern(p), sh.it.intern(h))).collect();
    let t = std::time::Instant::now();
    let (d, step) = scratch(&mut sh, &mem);
    let build_s = t.elapsed().as_secs_f64();
    let rows = edge_rows(&sh, &Below::default(), &d);
    let mut got = std::collections::BTreeMap::<(String, u32, String, u32, u32, u32), u32>::new();
    for r in &rows {
        *got.entry((sh.it.str(r.0).to_owned(), r.1, sh.it.str(r.3).to_owned(), r.4, r.5, r.8)).or_default() += 1;
    }
    let mut want_e = std::collections::BTreeMap::<(String, u32, String, u32, u32, u32), u32>::new();
    for l in std::fs::read_to_string(&native).unwrap().lines() {
        let f: Vec<&str> = l.split('\t').collect();
        *want_e.entry((f[0].to_owned(), f[1].parse().unwrap(), f[2].to_owned(), f[3].parse().unwrap(), f[4].parse().unwrap(), f[5].parse().unwrap())).or_default() += 1;
    }
    let missing: Vec<_> = want_e.iter().filter(|(k, c)| got.get(*k) != Some(c)).map(|(k, c)| (k.clone(), *c, got.get(k).copied().unwrap_or(0))).collect();
    let extra: Vec<_> = got.iter().filter(|(k, _)| !want_e.contains_key(*k)).map(|(k, c)| (k.clone(), *c)).collect();
    let n_rows = rows.len();
    drop(rows);
    let base = d.freeze(false);
    let fp_built = replay::mem_json();
    let res = serde_json::json!({
        "members": ms.len(), "summaries_loaded": n, "load_s": load_s, "build_s": build_s,
        "edges_rows": n_rows, "edges_native": want_e.values().sum::<u32>(), "edges_native_distinct": want_e.len(), "edges_got_distinct": got.len(),
        "missing": missing.len(), "extra": extra.len(),
        "first_missing": missing.iter().take(20).map(|x| format!("{:?}", x)).collect::<Vec<_>>(),
        "first_extra": extra.iter().take(20).map(|x| format!("{:?}", x)).collect::<Vec<_>>(),
        "counters": base.counters, "anomalies": sh.anomalies, "step": step,
        "bytes": base.bytes(), "interner": {"strings": sh.it.len(), "bytes": sh.it.bytes()},
        "qtable_bytes": sh.qt.bytes(), "deftable_bytes": sh.defs.bytes(), "summaries_bytes": sh.sums.bytes(),
        "mem": {"loaded": fp_loaded, "built": fp_built},
        "memo_kinds": replay::memo_kinds(&sh, &base),
    });
    std::fs::write(out_dir.join("parity.json"), serde_json::to_string_pretty(&res).unwrap()).unwrap();
    println!("{}", serde_json::to_string_pretty(&res).unwrap());
    if !missing.is_empty() || !extra.is_empty() {
        std::process::exit(1);
    }
}
