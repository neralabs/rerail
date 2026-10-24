//! `facts-b ygraph replay`: the base graph over the strict-majority membership, the 40
//! worktrees as deltas, S3's events maintained one at a time, each (or every N-th) checked
//! against a from-scratch build of that worktree in a forked child.
use super::engine::{self, KIND_NAMES, NK};
use super::view::{assign_def_ids, Graph, GraphRead, GraphView};
use super::world::*;
use super::{arg, bench, read_members, scratch, shared};
use rustc_hash::FxHashMap;
use serde_json::json;
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

pub fn memo_kinds(sh: &Shared, base: &Base) -> serde_json::Value {
    let mut n = [0usize; NK];
    let mut b = [0usize; NK];
    for (id, m) in &base.memos.memos {
        let k = sh.qt.qs[*id as usize].kind as usize;
        n[k] += 1;
        b[k] += m.bytes();
    }
    let mut out = serde_json::Map::new();
    for k in 0..NK {
        out.insert(KIND_NAMES[k].into(), json!({"memos": n[k], "bytes": b[k]}));
    }
    serde_json::Value::Object(out)
}

struct Event {
    seq: u64,
    worktree: u32,
    group: String,
    kind: String,
    changes: Vec<(String, Option<String>, Option<String>)>,
}

fn read_events(p: &std::path::Path) -> Vec<Event> {
    std::fs::read_to_string(p)
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let j: serde_json::Value = serde_json::from_str(l).unwrap();
            Event {
                seq: j["seq"].as_u64().unwrap(),
                worktree: j["worktree"].as_u64().unwrap() as u32,
                group: j["group"].as_str().unwrap().to_owned(),
                kind: j["kind"].as_str().unwrap_or("").to_owned(),
                changes: j["changes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|c| {
                        let c = c.as_array().unwrap();
                        (c[0].as_str().unwrap().to_owned(), c[1].as_str().map(str::to_owned), c[2].as_str().map(str::to_owned))
                    })
                    .collect(),
            }
        })
        .collect()
}

fn mb(b: usize) -> f64 {
    b as f64 / 1e6
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() as f64 - 1.0) * p).round() as usize]
}

fn summary(mut v: Vec<f64>) -> serde_json::Value {
    let n = v.len();
    let mean = if n == 0 { 0.0 } else { v.iter().sum::<f64>() / n as f64 };
    json!({"n": n, "mean": mean, "p50": pct(&mut v, 0.5), "p95": pct(&mut v, 0.95), "p99": pct(&mut v, 0.99), "max": pct(&mut v, 1.0)})
}

/// The membership-level difference of a worktree from the base: changes to apply.
fn diff(sh: &mut Shared, base: &FxHashMap<u32, u32>, w: &[(String, String)]) -> Vec<Change> {
    let wm: FxHashMap<u32, u32> = w.iter().map(|(p, h)| (sh.it.intern(p), sh.it.intern(h))).collect();
    let mut out = Vec::new();
    for (&p, &c) in &wm {
        if base.get(&p) != Some(&c) {
            out.push(Change { path: p, old: base.get(&p).copied(), new: Some(c) });
        }
    }
    for (&p, &c) in base {
        if !wm.contains_key(&p) {
            out.push(Change { path: p, old: Some(c), new: None });
        }
    }
    out.sort_by_key(|c| c.path);
    out
}

/// Divergence of a worktree: member paths whose content differs from the base.
fn divergence(_base: &Base, d: &Delta) -> usize {
    d.paths.len()
}

/// Exactness check of worktree `d` in a forked child: a from-scratch build of its members, its
/// edges (as `linked_edges` rows) and refusal/incomplete state compared with the maintained one.
fn check_child(sh: &mut Shared, base: &Base, d: &Delta, label: &str, out: &std::path::Path) -> ! {
    let t = Instant::now();
    let maintained = edge_rows(sh, base, d);
    let members: Vec<(u32, u32)> = d.members(base).into_iter().map(|(p, r)| (p, r.content)).collect();
    let (empty, sd, _) = scratch(sh, &members);
    let fresh = edge_rows(sh, &empty, &sd);
    let only_m: Vec<_> = maintained.iter().filter(|r| fresh.binary_search(r).is_err()).take(5).map(|r| format!("{} {} -> {} {}", sh.it.str(r.0), r.1, sh.it.str(r.3), r.4)).collect();
    let only_f: Vec<_> = fresh.iter().filter(|r| maintained.binary_search(r).is_err()).take(5).map(|r| format!("{} {} -> {} {}", sh.it.str(r.0), r.1, sh.it.str(r.3), r.4)).collect();
    let n_m = maintained.iter().filter(|r| fresh.binary_search(r).is_err()).count();
    let n_f = fresh.iter().filter(|r| maintained.binary_search(r).is_err()).count();
    let equal = n_m == 0 && n_f == 0 && d.counters == sd.counters;
    let line = json!({
        "at": label, "equal": equal, "edges": maintained.len(), "fresh_edges": fresh.len(),
        "maintained_only": n_m, "fresh_only": n_f, "first_maintained_only": only_m, "first_fresh_only": only_f,
        "counters": d.counters, "fresh_counters": sd.counters, "incomplete": d.incomplete(), "refused": d.refused(),
        "check_s": t.elapsed().as_secs_f64(),
    });
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(out).unwrap();
    f.write_all(format!("{line}\n").as_bytes()).unwrap();
    unsafe { libc::_exit(0) }
}

struct Checks {
    file: PathBuf,
    inflight: usize,
    max: usize,
    forked: usize,
}

impl Checks {
    fn reap(&mut self, block: bool) {
        while self.inflight > 0 {
            let mut st = 0;
            let r = unsafe { libc::waitpid(-1, &mut st, if block { 0 } else { libc::WNOHANG }) };
            if r <= 0 {
                break;
            }
            assert!(libc::WIFEXITED(st) && libc::WEXITSTATUS(st) == 0, "check child failed: status {st}");
            self.inflight -= 1;
            if !block {
                continue;
            }
            if self.inflight < self.max {
                break;
            }
        }
    }
    fn spawn(&mut self, sh: &mut Shared, base: &Base, d: &Delta, label: &str) {
        while self.inflight >= self.max {
            self.reap(true);
        }
        std::io::stdout().flush().ok();
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork");
        if pid == 0 {
            check_child(sh, base, d, label, &self.file);
        }
        self.inflight += 1;
        self.forked += 1;
        self.reap(false);
    }
    fn finish(&mut self) {
        while self.inflight > 0 {
            self.reap(true);
        }
    }
}

pub fn rss() -> (u64, u64) {
    crate::replay::rss()
}

#[repr(C)]
#[derive(Default)]
struct MallocStats {
    blocks_in_use: u32,
    size_in_use: usize,
    max_size_in_use: usize,
    size_allocated: usize,
}

extern "C" {
    fn malloc_zone_pressure_relief(zone: *mut libc::c_void, goal: usize) -> usize;
    fn malloc_zone_statistics(zone: *mut libc::c_void, stats: *mut MallocStats);
}

/// (heap bytes in use over every malloc zone, phys_footprint after returning free pages).
pub fn heap() -> (u64, u64) {
    let mut s = MallocStats::default();
    unsafe {
        malloc_zone_pressure_relief(std::ptr::null_mut(), 0);
        malloc_zone_statistics(std::ptr::null_mut(), &mut s);
    }
    (s.size_in_use as u64, rss().1)
}

pub fn mem_json() -> serde_json::Value {
    let (h, f) = heap();
    json!({"heap_mb": h as f64 / 1e6, "footprint_mb": f as f64 / 1e6})
}

pub fn run(args: &[String]) {
    let out_dir = PathBuf::from(arg(args, "--out").expect("--out DIR"));
    std::fs::create_dir_all(&out_dir).unwrap();
    let limit: Option<usize> = arg(args, "--limit").map(|x| x.parse().unwrap());
    let check_every: usize = arg(args, "--check-every").map_or(1, |x| x.parse().unwrap());
    let workers: usize = arg(args, "--workers").map_or(6, |x| x.parse().unwrap());
    let check_load = !args.iter().any(|a| a == "--no-check-load");
    let exports = PathBuf::from(arg(args, "--exports").unwrap_or_else(|| bench().join("phase-2/s3-crossfile/exports/w6/replay-dead-code.jsonl").display().to_string()));
    let events_p = PathBuf::from(arg(args, "--events").unwrap_or_else(|| bench().join("spike-b/edits/seed-5eedb/events.jsonl").display().to_string()));
    let mdir = bench().join("spike-b/membership");
    let mut checks = Checks { file: out_dir.join("checks.jsonl"), inflight: 0, max: workers, forked: 0 };
    let _ = std::fs::remove_file(&checks.file);

    let mut sh = shared();
    let t = Instant::now();
    let n_sums = super::summary::load(&exports, &mut sh.it, &mut sh.sums, None);
    let load_s = t.elapsed().as_secs_f64();
    let fp_inputs = mem_json();
    let inputs = json!({"summaries": n_sums, "summary_bytes": sh.sums.bytes(), "interner_strings": sh.it.len(), "interner_bytes": sh.it.bytes(), "load_s": load_s, "mem": fp_inputs});
    eprintln!("inputs: {inputs}");
    let it_before = sh.it.bytes();

    // base
    let base_m = read_members(&mdir.join("base.tsv"));
    let base_mem: Vec<(u32, u32)> = base_m.iter().map(|(p, h)| (sh.it.intern(p), sh.it.intern(h))).collect();
    let base_map: FxHashMap<u32, u32> = base_mem.iter().copied().collect();
    let t = Instant::now();
    let (_e, bd, bstep) = scratch(&mut sh, &base_mem);
    let base_build_s = t.elapsed().as_secs_f64();
    let base = bd.freeze();
    assign_def_ids(&mut sh, &base, None);
    let fp_base = mem_json();
    let bb = base.bytes();
    let base_info = json!({
        "members": base_m.len(), "build_s": base_build_s, "step": bstep, "bytes": bb,
        "memo_kinds": memo_kinds(&sh, &base), "counters": base.counters,
        "interner_strings": sh.it.len(), "interner_bytes": sh.it.bytes(), "interner_growth_bytes": sh.it.bytes() - it_before,
        "qtable_bytes": sh.qt.bytes(), "deftable_bytes": sh.defs.bytes(), "mem": fp_base,
    });
    eprintln!("base: {}", serde_json::to_string(&base_info).unwrap());

    // worktrees as deltas
    let mut wts: std::collections::BTreeMap<u32, Delta> = Default::default();
    let t = Instant::now();
    let mut load_steps = Vec::new();
    for w in 1..=40u32 {
        let m = read_members(&mdir.join(format!("{w:02}.tsv")));
        let ch = diff(&mut sh, &base_map, &m);
        let mut d = Delta::at_base(&base);
        let st = apply(&mut sh, &base, &mut d, &ch);
        load_steps.push(json!({"worktree": w, "changes": ch.len(), "ms": st.ms, "work": st.work, "rows_touched": st.table_rows_touched, "keys_changed": st.keys_changed, "bytes": d.bytes(), "breakdown": d.breakdown(), "divergence": divergence(&base, &d)}));
        if check_load {
            checks.spawn(&mut sh, &base, &d, &format!("load w{w:02}"));
        }
        wts.insert(w, d);
    }
    let deltas_s = t.elapsed().as_secs_f64();
    let fp_loaded = mem_json();
    let delta_bytes_loaded: usize = wts.values().map(Delta::bytes).sum();
    eprintln!("40 deltas: {deltas_s:.1}s, Σ delta {:.1} MB, mem {fp_loaded}", mb(delta_bytes_loaded));

    // the shared graph API, materialised once over the base
    let t = Instant::now();
    let (graph_s, graph_bytes, graph_edges) = {
        let g = Graph::build(&sh, &base);
        (t.elapsed().as_secs_f64(), g.bytes(), g.edge_count())
    };

    // events
    let events = read_events(&events_p);
    let events: Vec<Event> = match limit {
        Some(n) => events.into_iter().take(n).collect(),
        None => events,
    };
    let mut per = Vec::new();
    let mut lat_edit: Vec<f64> = Vec::new();
    let mut rows_edit: Vec<f64> = Vec::new();
    let mut rerun_edit: Vec<f64> = Vec::new();
    let mut probes_edit: Vec<f64> = Vec::new();
    let mut lat_other: Vec<f64> = Vec::new();
    let mut fallbacks = json!({"init": 0, "incomplete_flip": 0});
    let mut fp_samples = vec![json!({"at": "loaded", "mem": fp_loaded, "delta_bytes": delta_bytes_loaded, "interner_bytes": sh.it.bytes()})];
    for (i, ev) in events.iter().enumerate() {
        if ev.group == "new_worktree" || ev.group == "commit_init" {
            assert!(!wts.contains_key(&ev.worktree));
            wts.insert(ev.worktree, Delta::at_base(&base));
        }
        let changes: Vec<Change> = ev
            .changes
            .iter()
            .map(|(p, o, n)| Change { path: sh.it.intern(p), old: o.as_ref().map(|x| sh.it.intern(x)), new: n.as_ref().map(|x| sh.it.intern(x)) })
            .collect();
        let d = wts.get_mut(&ev.worktree).unwrap();
        let st = apply(&mut sh, &base, d, &changes);
        if st.init_fallback {
            fallbacks["init"] = json!(fallbacks["init"].as_u64().unwrap() + 1);
        }
        if st.incomplete_flip {
            fallbacks["incomplete_flip"] = json!(fallbacks["incomplete_flip"].as_u64().unwrap() + 1);
        }
        let single = ev.group == "edit" || ev.group == "new_worktree_edit";
        if single {
            lat_edit.push(st.ms);
            rows_edit.push((st.table_rows_touched + st.work.rerun + st.work.fresh) as f64);
            rerun_edit.push(st.work.rerun as f64);
            probes_edit.push(st.work.probes as f64);
        } else {
            lat_other.push(st.ms);
        }
        let d = &wts[&ev.worktree];
        per.push(json!({"seq": ev.seq, "worktree": ev.worktree, "group": ev.group, "kind": ev.kind, "step": st, "delta_bytes": d.bytes(), "divergence": divergence(&base, d)}));
        if (i + 1) % check_every == 0 {
            checks.spawn(&mut sh, &base, d, &format!("event {}", ev.seq));
        }
        if (i + 1) % 100 == 0 {
            let db: usize = wts.values().map(Delta::bytes).sum();
            fp_samples.push(json!({"at": format!("event {}", i + 1), "mem": mem_json(), "delta_bytes": db, "interner_bytes": sh.it.bytes()}));
            eprintln!("event {}: p50 {:.1} ms, Σ delta {:.1} MB, checks forked {}", i + 1, pct(&mut lat_edit.clone(), 0.5), mb(db), checks.forked);
        }
    }
    // a view per worktree at the end: materialisation cost and bytes
    let mut views = Vec::new();
    for d in wts.values() {
        assign_def_ids(&mut sh, &base, Some(d));
    }
    {
        let shr: &Shared = &sh;
        let g = Graph::build(shr, &base);
        for (w, d) in &wts {
            let t = Instant::now();
            let v = GraphView::build(&g, shr, &base, d);
            let ms = t.elapsed().as_secs_f64() * 1e3;
            // smoke: every view module's edges resolve to defs of the view
            let mut dangling = 0usize;
            for (p, r) in d.members(&base) {
                if !r.linked {
                    continue;
                }
                for dd in v.definitions(p) {
                    for e in v.edges_from(p, dd.name) {
                        if v.def(e.dst_def).is_none() {
                            dangling += 1;
                        }
                    }
                }
            }
            views.push(json!({"worktree": w, "build_ms": ms, "view_bytes": v.bytes(), "delta_bytes": d.bytes(), "divergence": divergence(&base, d), "epoch": v.epoch(), "dangling": dangling, "incomplete": v.incomplete(), "refused": v.refused()}));
        }
    }
    checks.finish();
    let db_end: usize = wts.values().map(Delta::bytes).sum();
    fp_samples.push(json!({"at": "end", "mem": mem_json(), "delta_bytes": db_end, "interner_bytes": sh.it.bytes()}));
    // exactness summary
    let lines: Vec<serde_json::Value> = std::fs::read_to_string(&checks.file).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let unequal: Vec<&serde_json::Value> = lines.iter().filter(|l| !l["equal"].as_bool().unwrap()).collect();
    let exactness = json!({"checks": lines.len(), "forked": checks.forked, "unequal": unequal.len(), "first_unequal": unequal.iter().take(5).collect::<Vec<_>>()});
    let res = json!({
        "inputs": inputs, "base": base_info, "graph": {"build_s": graph_s, "bytes": graph_bytes, "edges": graph_edges},
        "load": {"deltas_s": deltas_s, "steps": load_steps, "delta_bytes": delta_bytes_loaded},
        "latency_single_file_ms": summary(lat_edit), "touched_single_file": summary(rows_edit), "rerun_single_file": summary(rerun_edit),
        "probes_single_file": summary(probes_edit), "latency_other_ms": summary(lat_other),
        "fallbacks": fallbacks, "footprint": fp_samples, "views": views, "exactness": exactness, "anomalies": sh.anomalies,
        "events": per.len(), "per_event": per, "qtable": sh.qt.qs.len(), "defs": sh.defs.defs.len(),
    });
    std::fs::write(out_dir.join("results.json"), serde_json::to_string_pretty(&res).unwrap()).unwrap();
    let mut short = res.clone();
    short.as_object_mut().unwrap().remove("per_event");
    short["load"].as_object_mut().unwrap().remove("steps");
    println!("{}", serde_json::to_string_pretty(&short).unwrap());
    let _ = engine::HMAX;
    if !unequal.is_empty() {
        std::process::exit(1);
    }
}
