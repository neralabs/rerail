//! `lattice run`: the fixpoint over the mapped columns of `lattice build`, timed, counted and
//! checked against Y2's reached states and levels.
//!
//! - One graph (`--fleet` absent): the first worktree's (the base) as a flat CSR. Workload A.
//! - `--fleet within`: every worktree in turn (base + its replacement rows), each fixpoint on
//!   `--threads` workers. `--fleet across`: `--threads` workers, each taking whole worktrees and
//!   running the sequential kernel. Workload B.
//!
//! Timed runs do nothing but the fixpoints; the checks (every reached id and level against Y2's,
//! the values' digest) run afterwards, untimed: on every run's result for one graph, on one
//! further pass of the same mode for a fleet.
use super::cols::{self, Col};
use super::round::{self, Counts, Csr, Lattice, Layered, Rows, Values, INF, SPARSE_DIV};
use serde_json::json;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

struct Graph {
    n: usize,
    meta: serde_json::Value,
    base_off: Col<u32>,
    base_tgt: Col<u32>,
    ov_bits: Col<u64>,
    ov_rank: Col<u32>,
    ov_off: Col<u32>,
    ov_tgt: Col<u32>,
    seeds: Col<u32>,
    y2_id: Col<u32>,
    y2_lvl: Col<u32>,
    /// The base's transpose (mapped for `--kernel dir` only).
    rev: Option<(Col<u32>, Col<u32>)>,
}

fn range(v: &serde_json::Value) -> std::ops::Range<usize> {
    let a = v[0].as_u64().unwrap() as usize;
    a..a + v[1].as_u64().unwrap() as usize
}

impl Graph {
    fn load(dir: &Path, rev: bool) -> Graph {
        let meta: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("meta.json")).unwrap()).unwrap();
        Graph {
            n: meta["n"].as_u64().unwrap() as usize,
            base_off: cols::map(&dir.join("base_off.u32")),
            base_tgt: cols::map(&dir.join("base_tgt.u32")),
            ov_bits: cols::map(&dir.join("ov_bits.u64")),
            ov_rank: cols::map(&dir.join("ov_rank.u32")),
            ov_off: cols::map(&dir.join("ov_off.u32")),
            ov_tgt: cols::map(&dir.join("ov_tgt.u32")),
            seeds: cols::map(&dir.join("seeds.u32")),
            y2_id: cols::map(&dir.join("y2_id.u32")),
            y2_lvl: cols::map(&dir.join("y2_lvl.u32")),
            rev: rev.then(|| (cols::map(&dir.join("rev_off.u32")), cols::map(&dir.join("rev_tgt.u32")))),
            meta,
        }
    }
    fn worktrees(&self) -> usize {
        self.meta["worktrees"].as_array().unwrap().len()
    }
    fn wt(&self, w: usize) -> &serde_json::Value {
        &self.meta["worktrees"][w]
    }
    fn base(&self) -> Csr<'_> {
        Csr::new(&self.base_off, &self.base_tgt, self.n)
    }
    fn layered<'a>(&'a self, w: usize, base: Csr<'a>) -> Layered<'a> {
        let l = &self.wt(w)["layer"];
        let delta = Csr::new(&self.ov_off[range(&l["off"])], &self.ov_tgt[range(&l["tgt"])], self.n);
        let b = range(&l["bits"]);
        Layered::new(base, &self.ov_bits[b.clone()], &self.ov_rank[b], delta)
    }
    fn seeds(&self, w: usize) -> &[u32] {
        &self.seeds[range(&self.wt(w)["seeds"])]
    }
    fn y2(&self, w: usize) -> (&[u32], &[u32]) {
        let r = range(&self.wt(w)["y2_levels"]);
        (&self.y2_id[r.clone()], &self.y2_lvl[r])
    }
}

/// Differences from Y2's reached states (and levels, for `MinLevel`): (count, first few).
fn check(v: &Values, n: usize, ids: &[u32], lvls: &[u32]) -> (usize, Vec<String>) {
    let mut diffs = 0;
    let mut first = Vec::new();
    let mut j = 0;
    for x in 0..n as u32 {
        let want = if j < ids.len() && ids[j] == x {
            j += 1;
            Some(lvls[j - 1])
        } else {
            None
        };
        let got = match v {
            Values::Levels(l) => (l[x as usize] != INF).then_some(l[x as usize]),
            Values::Bits(_) => v.reached(x).then_some(want.unwrap_or(0)),
        };
        if got != want {
            diffs += 1;
            if first.len() < 10 {
                first.push(format!("id {x}: lattice {got:?}, y2 {want:?}"));
            }
        }
    }
    (diffs, first)
}

fn digest(v: &Values) -> String {
    blake3::hash(v.bytes()).to_hex()[..16].to_owned()
}

fn fixpoint<R: Rows>(g: &R, rev: Option<&Csr<'_>>, seeds: &[u32], lat: Lattice, kernel: &str, threads: usize) -> (Values, Counts) {
    match kernel {
        "seq" => {
            assert_eq!(threads, 1, "--kernel seq runs on one thread");
            round::sequential(g, seeds, lat)
        }
        "par" => round::parallel(g, seeds, lat, threads),
        "edge" => round::parallel_edges(g, seeds, lat, threads),
        "own" => round::parallel_owner(g, seeds, lat, threads),
        "dir" => round::parallel_dir(g, rev.expect("--kernel dir: the base worktree only"), seeds, lat, threads),
        k => panic!("--kernel {k}: seq | par | edge | own | dir"),
    }
}

fn median(v: &[f64]) -> f64 {
    let mut v = v.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    if v.len() % 2 == 1 {
        v[v.len() / 2]
    } else {
        (v[v.len() / 2 - 1] + v[v.len() / 2]) / 2.0
    }
}

fn counts_json(c: &Counts) -> serde_json::Value {
    json!({"seeds": c.seeds, "rounds": c.rounds.len(), "edges_scanned": c.edges(), "max_width": c.rounds.iter().map(|r| r.width).max().unwrap_or(0),
           "width": c.rounds.iter().map(|r| r.width).collect::<Vec<_>>(), "edges": c.rounds.iter().map(|r| r.edges).collect::<Vec<_>>(),
           "changed": c.rounds.iter().map(|r| r.changed).collect::<Vec<_>>(), "list_rounds": c.list_rounds,
           "pull": c.rounds.iter().map(|r| r.pull).collect::<Vec<_>>()})
}

/// Private bytes of one fixpoint's own state: values, two frontier bitsets, and for the
/// sequential kernel its two sparse lists.
fn fixpoint_bytes(n: usize, lat: Lattice, kernel: &str) -> usize {
    let vals = match lat {
        Lattice::MinLevel => n * 4,
        Lattice::Set => n.div_ceil(64) * 8,
    };
    vals + 2 * n.div_ceil(64) * 8 + if kernel == "seq" { 2 * (n / SPARSE_DIV).max(1) * 4 } else { 0 }
}

pub struct Opts {
    pub graph: std::path::PathBuf,
    pub out: std::path::PathBuf,
    pub threads: usize,
    pub runs: usize,
    pub lattice: Lattice,
    pub kernel: String,
    pub fleet: Option<String>,
}

pub fn run(o: Opts) {
    let g = Graph::load(&o.graph, o.kernel == "dir");
    let t = Instant::now();
    let base = g.base();
    let validate_ms = t.elapsed().as_secs_f64() * 1e3;
    let mem0 = cols::memory();
    let mut res = json!({"graph": o.graph.display().to_string(), "n": g.n, "threads": o.threads, "runs": o.runs, "lattice": o.lattice, "kernel": o.kernel, "fleet": o.fleet,
        "validate_ms": validate_ms, "memory_before": mem0, "pid": std::process::id()});
    match o.fleet.as_deref() {
        None => single(&g, base, &o, &mut res),
        Some(m) => fleet(&g, base, &o, m, &mut res),
    }
    let (maxrss, resident, footprint, peak_footprint) = cols::memory();
    res["memory"] = json!({"max_rss": maxrss, "resident": resident, "footprint": footprint, "peak_footprint": peak_footprint,
        "mapped_bytes": cols::MAPPED.load(Relaxed), "copied_bytes": cols::COPIED.load(Relaxed)});
    std::fs::write(&o.out, serde_json::to_string_pretty(&res).unwrap()).unwrap();
    eprintln!("lattice run: {}", serde_json::to_string(&json!({"wall_ms_median": res["wall_ms_median"], "cpu_ms_median": res["cpu_ms_median"], "exact": res["exact"], "rounds": res["counts"]["rounds"], "memory": res["memory"]})).unwrap());
}

fn single(g: &Graph, base: Csr<'_>, o: &Opts, res: &mut serde_json::Value) {
    let seeds = g.seeds(0);
    let (ids, lvls) = g.y2(0);
    let rev = g.rev.as_ref().map(|(off, tgt)| Csr::new(off, tgt, g.n));
    let (mut wall, mut cpu, mut la, mut digests, mut waits, mut round_ms, mut start_ms) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut counts: Option<Counts> = None;
    let mut exact = true;
    let mut first_diffs = Vec::new();
    for _ in 0..o.runs {
        la.push(cols::load_average()[0]);
        let c0 = cols::cpu_seconds();
        let t0 = Instant::now();
        let (v, c) = fixpoint(&base, rev.as_ref(), seeds, o.lattice, &o.kernel, o.threads);
        wall.push(t0.elapsed().as_secs_f64() * 1e3);
        cpu.push((cols::cpu_seconds() - c0) * 1e3);
        let (d, f) = check(&v, g.n, ids, lvls);
        exact &= d == 0;
        if d > 0 && first_diffs.is_empty() {
            first_diffs = f;
        }
        digests.push(digest(&v));
        waits.push(c.barrier_wait_ms.clone());
        round_ms.push(c.round_ms.clone());
        start_ms.push(c.start_ms);
        match &counts {
            None => counts = Some(c),
            Some(c1) => assert_eq!(c1.algebraic(), c.algebraic(), "counts differ between runs"),
        }
    }
    let c = counts.unwrap();
    res["wall_ms"] = json!(wall);
    res["cpu_ms"] = json!(cpu);
    res["wall_ms_median"] = json!(median(&wall));
    res["cpu_ms_median"] = json!(median(&cpu));
    res["load_average_1m"] = json!(la);
    res["exact"] = json!(exact);
    res["first_differences"] = json!(first_diffs);
    res["y2_reached"] = json!(ids.len());
    res["digests"] = json!(digests);
    res["counts"] = counts_json(&c);
    res["barrier_wait_ms"] = json!(waits);
    res["round_ms"] = json!(round_ms);
    res["start_ms"] = json!(start_ms);
    res["fixpoint_private_bytes"] = json!(fixpoint_bytes(g.n, o.lattice, &o.kernel));
    res["worker_alloc_bytes"] = json!(c.worker_alloc_bytes);
}

fn fleet(g: &Graph, base: Csr<'_>, o: &Opts, mode: &str, res: &mut serde_json::Value) {
    let w = g.worktrees();
    let graphs: Vec<Layered<'_>> = (0..w).map(|i| g.layered(i, base)).collect();
    // One pass over every worktree; `on` sees each worktree's result (untimed when checking).
    let pass = |check_each: bool| -> (f64, Vec<f64>, Vec<Option<(Counts, usize, String)>>) {
        let per = parking_lot::Mutex::new(vec![0f64; w]);
        let out: parking_lot::Mutex<Vec<Option<(Counts, usize, String)>>> = parking_lot::Mutex::new((0..w).map(|_| None).collect());
        let finish = |i: usize, v: Values, c: Counts| {
            if check_each {
                let (ids, lvls) = g.y2(i);
                let (d, _) = check(&v, g.n, ids, lvls);
                out.lock()[i] = Some((c, d, digest(&v)));
            }
        };
        let t0 = Instant::now();
        match mode {
            "within" => {
                for i in 0..w {
                    let t1 = Instant::now();
                    let (v, c) = fixpoint(&graphs[i], None, g.seeds(i), o.lattice, &o.kernel, o.threads);
                    per.lock()[i] = t1.elapsed().as_secs_f64() * 1e3;
                    finish(i, v, c);
                }
            }
            "across" => {
                let next = AtomicUsize::new(0);
                std::thread::scope(|s| {
                    for _ in 0..o.threads {
                        s.spawn(|| loop {
                            let i = next.fetch_add(1, Relaxed);
                            if i >= w {
                                break;
                            }
                            let t1 = Instant::now();
                            let (v, c) = round::sequential(&graphs[i], g.seeds(i), o.lattice);
                            per.lock()[i] = t1.elapsed().as_secs_f64() * 1e3;
                            finish(i, v, c);
                        });
                    }
                });
            }
            m => panic!("--fleet {m}: within | across"),
        }
        (t0.elapsed().as_secs_f64() * 1e3, per.into_inner(), out.into_inner())
    };
    let (mut wall, mut cpu, mut la, mut per_runs) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for _ in 0..o.runs {
        la.push(cols::load_average()[0]);
        let c0 = cols::cpu_seconds();
        let (ms, per, _) = pass(false);
        cpu.push((cols::cpu_seconds() - c0) * 1e3);
        wall.push(ms);
        per_runs.push(per);
    }
    let (_, _, checked) = pass(true);
    let mut exact = true;
    let mut wts = Vec::new();
    for (i, x) in checked.into_iter().enumerate() {
        let (c, d, dg) = x.unwrap();
        exact &= d == 0;
        let per: Vec<f64> = per_runs.iter().map(|p| p[i]).collect();
        wts.push(json!({"worktree": g.wt(i)["worktree"], "differences": d, "digest": dg, "y2_reached": g.y2(i).0.len(), "fixpoint_ms_median": median(&per), "counts": counts_json(&c)}));
    }
    let kernel = if mode == "across" { "seq" } else { o.kernel.as_str() };
    res["wall_ms"] = json!(wall);
    res["cpu_ms"] = json!(cpu);
    res["wall_ms_median"] = json!(median(&wall));
    res["cpu_ms_median"] = json!(median(&cpu));
    res["load_average_1m"] = json!(la);
    res["exact"] = json!(exact);
    res["worktrees"] = json!(wts);
    res["fixpoint_private_bytes"] = json!(fixpoint_bytes(g.n, o.lattice, kernel));
    res["private_bytes_per_worker"] = json!(if mode == "across" { fixpoint_bytes(g.n, o.lattice, "seq") } else { 0 });
}
