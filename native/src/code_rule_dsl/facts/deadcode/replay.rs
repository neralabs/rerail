//! The dead-code families on facts-engine E3's replay (`facts-b lattice maint`, report
//! docs/facts/reports/lattice-e3-maint.md): the same inputs (X6's per-content exports, Spike B's
//! 40 memberships and Y3's layers over them, the 1,111 events of `seed-5eedb`), each event pushed
//! through `propagate` on the store or the overlay, the reached set maintained or recomputed.
//! Ignored (minutes, gigabytes, data outside the repository); run one at a time:
//!
//! ```text
//! cargo test --profile profiling --lib --features metal-tests -- --ignored --nocapture deadcode::replay::<test>
//! ```
//!
//! Environment: `RERAIL_DEADCODE_MODE` = `maintained` (default) | `recompute` | `recompute:T` (T
//! threads); `RERAIL_DEADCODE_GRAPH` = `store` (default) | `overlay`; `RERAIL_DEADCODE_LIMIT`
//! (events); `RERAIL_DEADCODE_CHECK_EVERY` (events between checks; 0: none);
//! `RERAIL_DEADCODE_CHECK_LOAD` (1: check every created worktree); `RERAIL_DEADCODE_WORKERS`
//! (forked checks at once, 8); `RERAIL_DEADCODE_OUT` (run directory; default
//! `~/rerail-bench/final-shape/crossfile/runs/<time>-<label>`); data:
//! `RERAIL_DEADCODE_EXPORTS`, `RERAIL_DEADCODE_EVENTS`, `RERAIL_DEADCODE_MEMBERS`,
//! `RERAIL_DEADCODE_ORACLE_DUMP`, with E3's defaults under `~/rerail-bench/facts`.
use super::*;
use crate::code_rule_dsl::facts::lattice::cols;
use crate::code_rule_dsl::facts::lmap::LMap;
use crate::code_rule_dsl::facts::overlay::{Limits, Overlay};
use crate::code_rule_dsl::facts::y2::dc::Rule;
use crate::code_rule_dsl::facts::y3::sym::SymState;
use crate::code_rule_dsl::facts::y3::{read_events, wire::Shared};
use crate::code_rule_dsl::facts::ygraph::replay::{plurality, CLUSTERS};
use crate::code_rule_dsl::facts::ygraph::world::Delta;
use crate::code_rule_dsl::facts::ygraph::{read_members, run_big, shared as y1_shared};
use crate::code_rule_dsl::prof::{footprint_mb, max_footprint_mb};
use crate::content_store::ContentStore;
use serde_json::{json, Value as Json};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn bench() -> PathBuf {
    crate::code_rule_dsl::facts::ygraph::bench()
}

fn path_env(name: &str, default: &str) -> PathBuf {
    env(name).map_or_else(|| bench().join(default), PathBuf::from)
}

fn mode() -> Mode {
    match env("RERAIL_DEADCODE_MODE").as_deref() {
        None | Some("maintained") => Mode::Maintained,
        Some("recompute") => Mode::Recompute { threads: 1 },
        Some(m) => match m.strip_prefix("recompute:").and_then(|t| t.parse().ok()) {
            Some(threads) => Mode::Recompute { threads },
            None => panic!("RERAIL_DEADCODE_MODE={m}: maintained | recompute | recompute:T"),
        },
    }
}

fn mode_label(m: Mode) -> String {
    match m {
        Mode::Maintained => "maintained".into(),
        Mode::Recompute { threads: 1 } => "recompute".into(),
        Mode::Recompute { threads } => format!("recompute{threads}"),
    }
}

fn load_average() -> f64 {
    cols::load_average()[0]
}

fn run_dir(label: &str) -> PathBuf {
    env("RERAIL_DEADCODE_OUT").map_or_else(
        || {
            let out = std::process::Command::new("date").arg("+%Y%m%d-%H%M%S").output().unwrap();
            let ts = String::from_utf8(out.stdout).unwrap();
            PathBuf::from(std::env::var("HOME").unwrap()).join("rerail-bench/final-shape/crossfile/runs").join(format!("{}-{label}", ts.trim()))
        },
        PathBuf::from,
    )
}

/// Percentiles as E3's summary (`e3_summary.py`, Y3's `pct`): sorted, index round((n - 1) p).
fn summary(v: &[f64]) -> Json {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p = |q: f64| if s.is_empty() { 0.0 } else { s[((s.len() - 1) as f64 * q).round() as usize] };
    json!({"n": s.len(), "p50": p(0.5), "p95": p(0.95), "p99": p(0.99), "max": p(1.0), "mean": s.iter().sum::<f64>() / s.len().max(1) as f64})
}

/// Forked checks, as E3's: the child runs `DeadCode::check` on a copy of the process and appends
/// one line to `file`.
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
            if block && self.inflight < self.max {
                break;
            }
        }
    }

    fn spawn<I: Inputs>(&mut self, dc: &DeadCode<I>, w: u32, label: &str) {
        while self.inflight >= self.max {
            self.reap(true);
        }
        std::io::stdout().flush().ok();
        std::io::stderr().flush().ok();
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork");
        if pid == 0 {
            let t = Instant::now();
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| dc.check(w)));
            let diffs = r.unwrap_or_else(|_| vec!["check panicked".to_owned()]);
            let line = json!({"equal": diffs.is_empty(), "differences": diffs, "worktree": w, "at": label, "ids": dc.reach(w).ext.n(), "check_s": t.elapsed().as_secs_f64()});
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&self.file).unwrap();
            f.write_all(format!("{line}\n").as_bytes()).unwrap();
            unsafe { libc::_exit(0) }
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

/// The run: Y3's world as E3 builds it, the lattice base from worktree 1, the 40 loads, the
/// events; per event the producers' work, `propagate`'s counts and the push's wall time.
fn replay(label: &str, default_check_every: usize, default_check_load: bool) -> Json {
    let mode = mode();
    let graph_kind = env("RERAIL_DEADCODE_GRAPH").unwrap_or_else(|| "store".into());
    let limit: usize = env("RERAIL_DEADCODE_LIMIT").map_or(usize::MAX, |x| x.parse().unwrap());
    let check_every: usize = env("RERAIL_DEADCODE_CHECK_EVERY").map_or(default_check_every, |x| x.parse().unwrap());
    let check_load = env("RERAIL_DEADCODE_CHECK_LOAD").map_or(default_check_load, |x| x == "1");
    let workers: usize = env("RERAIL_DEADCODE_WORKERS").map_or(8, |x| x.parse().unwrap());
    let out = run_dir(&format!("{label}-{}-{graph_kind}", mode_label(mode)));
    std::fs::create_dir_all(&out).unwrap();
    let exports = path_env("RERAIL_DEADCODE_EXPORTS", "explore/x6-rules/exports/replay-symbols-merged.jsonl");
    let events_p = path_env("RERAIL_DEADCODE_EVENTS", "spike-b/edits/seed-5eedb/events.jsonl");
    let mdir = path_env("RERAIL_DEADCODE_MEMBERS", "spike-b/membership");
    eprintln!("deadcode replay: {} into {}", mode_label(mode), out.display());
    let load_start = load_average();
    let (cpu0, t_all) = (cols::cpu_seconds(), Instant::now());
    let fp_start = footprint_mb();
    let store = Arc::new(ContentStore::open(&out.join("store.sqlite")).unwrap());

    // Y3's world, as E3 builds it: the base layer (strict-majority membership) and an overlay per
    // cluster (plurality membership).
    let mut sh = Shared { y1: y1_shared(), sym: Default::default(), y2_path: Default::default(), source: None, inputs_ms: 0.0 };
    crate::code_rule_dsl::facts::ygraph::summary::load(&exports, &mut sh.y1.it, &mut sh.y1.sums, None);
    crate::code_rule_dsl::facts::y3::input::load(&exports, &mut sh.sym.blocks, None);
    let fp_inputs = footprint_mb();
    let base_m = read_members(&mdir.join("base.tsv"));
    let (layer0, _) = wire::base_layer(&mut sh, &base_m);
    let mut world = World { sh, layers: vec![layer0], wts: BTreeMap::new() };
    let ms: Vec<Vec<(String, String)>> = (1..=40u32).map(|w| read_members(&mdir.join(format!("{w:02}.tsv")))).collect();
    for &(lo, hi) in &CLUSTERS {
        let cm = plurality(&ms, lo, hi);
        let World { sh, layers, .. } = &mut world;
        let b = below(layers, 0);
        let (top, _) = wire::over(sh, &b, &layers[0], &cm);
        let layer = top.freeze(true);
        world.layers.push(layer);
    }
    let world_s = t_all.elapsed().as_secs_f64();
    let fp_world = footprint_mb();
    eprintln!("deadcode replay: Y3 world: base + {} overlays in {world_s:.1}s", CLUSTERS.len());
    let layer_of = |w: u32| CLUSTERS.iter().position(|&(lo, hi)| lo <= w && w <= hi).map_or(0, |c| c + 1);
    let st1 = world.create(1, layer_of(1), &ms[0]);
    let (base, mut base_info) = Base::build(&world.wts[&1].top.facts, &out.join("base"));
    base_info["y3_create_ms"] = json!(st1.ms);
    let mut dc = DeadCode::new(world, base, mode);
    let fp_base = footprint_mb();
    eprintln!("deadcode replay: base {base_info}");

    let overlay = (graph_kind == "overlay").then(|| {
        let limits = Limits { top: 1 << 20, resident: 64 << 20, behind: 8, spill: out.join("overlay-spill"), background: true };
        Overlay::new(Arc::clone(&store), limits).unwrap()
    });
    assert!(overlay.is_some() || graph_kind == "store", "RERAIL_DEADCODE_GRAPH={graph_kind}: store | overlay");
    let graph: &dyn Graph = match &overlay {
        Some(o) => o,
        None => &*store,
    };
    let mut checks = Checks { file: out.join("checks.jsonl"), inflight: 0, max: workers, forked: 0 };
    let _ = std::fs::remove_file(&checks.file);
    let mut parent_diffs: Vec<Json> = Vec::new();
    let mut bindings_checked = 0usize;
    let mut check_bindings = |dc: &DeadCode<World>, w: u32, at: &str, diffs: &mut Vec<Json>| {
        bindings_checked += 1;
        let d = dc.bindings_differ(graph, w).unwrap();
        if !d.is_empty() {
            diffs.push(json!({"worktree": w, "at": at, "differences": d}));
        }
    };

    let mut loads = Vec::new();
    for w in 1..=40u32 {
        let y3_ms = if w > 1 { dc.inputs.create(w, layer_of(w), &ms[w as usize - 1]).ms } else { st1.ms };
        dc.step = Step::default();
        let t = Instant::now();
        dc.create(graph, w).unwrap();
        loads.push(json!({"worktree": w, "y3_ms": y3_ms, "create_ms": t.elapsed().as_secs_f64() * 1e3, "step": dc.step}));
        if check_load {
            check_bindings(&dc, w, &format!("load w{w:02}"), &mut parent_diffs);
            checks.spawn(&dc, w, &format!("load w{w:02}"));
        }
    }
    let loaded_s = t_all.elapsed().as_secs_f64();
    let mut samples = vec![
        json!({"at": "start", "footprint_mb": fp_start}),
        json!({"at": "per-content exports loaded (Y1 summaries, symbol blocks)", "footprint_mb": fp_inputs}),
        json!({"at": "Y3 world (base layer and 8 overlays)", "footprint_mb": fp_world}),
        json!({"at": "worktree 1 and the lattice base", "footprint_mb": fp_base}),
        json!({"at": "loaded", "footprint_mb": footprint_mb(), "worktrees": dc.worktrees().map(|w| dc.memory(w)).collect::<Vec<_>>()}),
    ];
    eprintln!("deadcode replay: 40 worktrees loaded at {loaded_s:.1}s, footprint {} MB", footprint_mb());

    let events = read_events(&events_p);
    let events: Vec<_> = events.into_iter().take(limit).collect();
    let (cpu1, t_ev) = (cols::cpu_seconds(), Instant::now());
    let mut per_event = Vec::new();
    for (i, ev) in events.iter().enumerate() {
        let mut create = None;
        if ev.group == "new_worktree" || ev.group == "commit_init" {
            let y3 = dc.inputs.create(ev.worktree, 0, &base_m);
            dc.step = Step::default();
            let t = Instant::now();
            dc.create(graph, ev.worktree).unwrap();
            create = Some(json!({"y3_ms": y3.ms, "ms": t.elapsed().as_secs_f64() * 1e3, "step": dc.step}));
        }
        let la = load_average();
        dc.step = Step::default();
        let t = Instant::now();
        let work = dc.push(graph, ev.worktree, &ev.changes, 64).unwrap();
        let total_ms = t.elapsed().as_secs_f64() * 1e3;
        let s = &dc.step;
        let producers = s.y1_ms + s.refs_ms + s.sym_ms + s.scope_ms + s.apply_ms + s.rows.ms + s.values.ms + s.findings_ms;
        per_event.push(json!({
            "i": i, "seq": ev.seq, "worktree": ev.worktree, "group": ev.group, "kind": ev.kind, "paths": ev.changes.len(), "load_1m": la,
            "total_ms": total_ms, "producers_ms": producers, "overhead_ms": total_ms - producers,
            "work": {"waves": work.waves, "rerun": work.rerun, "changed": work.changed}, "step": dc.step, "create": create,
            "footprint_mb": footprint_mb(),
        }));
        if check_every > 0 && (i + 1) % check_every == 0 {
            check_bindings(&dc, ev.worktree, &format!("event {}", ev.seq), &mut parent_diffs);
            checks.spawn(&dc, ev.worktree, &format!("event {}", ev.seq));
        }
        if (i + 1) % 250 == 0 {
            eprintln!("deadcode replay: event {} at {:.1}s, checks forked {}, footprint {} MB", i + 1, t_all.elapsed().as_secs_f64(), checks.forked, footprint_mb());
        }
        if (i + 1) % 500 == 0 {
            samples.push(json!({"at": format!("event {}", i + 1), "footprint_mb": footprint_mb()}));
        }
    }
    let events_s = t_ev.elapsed().as_secs_f64();
    let events_cpu = cols::cpu_seconds() - cpu1;
    if let Some(o) = &overlay {
        o.flush().unwrap();
    }
    checks.finish();
    samples.push(json!({"at": "end", "footprint_mb": footprint_mb(), "worktrees": dc.worktrees().map(|w| dc.memory(w)).collect::<Vec<_>>()}));
    let lines: Vec<Json> = std::fs::read_to_string(&checks.file).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let unequal: Vec<&Json> = lines.iter().filter(|l| !l["equal"].as_bool().unwrap()).collect();

    // Single-file edits (E3's population): per component, per count.
    let single: Vec<&Json> = per_event.iter().filter(|e| matches!(e["group"].as_str(), Some("edit" | "new_worktree_edit"))).collect();
    let f = |k: &dyn Fn(&Json) -> f64| summary(&single.iter().map(|e| k(e)).collect::<Vec<_>>());
    let st = |e: &Json, k: &str| e["step"][k].as_f64().unwrap_or(0.0);
    let latency = json!({
        "y1_ms": f(&|e| st(e, "y1_ms")),
        "refs_ms": f(&|e| st(e, "refs_ms")),
        "symbols_ms": f(&|e| st(e, "sym_ms")),
        "scope_ms": f(&|e| st(e, "scope_ms")),
        "apply_ms": f(&|e| st(e, "apply_ms")),
        "rows_ms": f(&|e| e["step"]["rows"]["ms"].as_f64().unwrap()),
        "rows_succ_ms": f(&|e| e["step"]["rows"]["succ_ms"].as_f64().unwrap()),
        "rows_map_ms": f(&|e| e["step"]["rows"]["map_ms"].as_f64().unwrap()),
        "rows_layer_ms": f(&|e| e["step"]["rows"]["layer_ms"].as_f64().unwrap()),
        "values_ms": f(&|e| e["step"]["values"]["ms"].as_f64().unwrap()),
        "csr_rows_ms (scope + rows, E3's)": f(&|e| st(e, "scope_ms") + e["step"]["rows"]["ms"].as_f64().unwrap()),
        "csr_rows_plus_values_ms (E3's H7 sum)": f(&|e| st(e, "scope_ms") + e["step"]["rows"]["ms"].as_f64().unwrap() + e["step"]["values"]["ms"].as_f64().unwrap()),
        "findings_ms": f(&|e| st(e, "findings_ms")),
        "digest_ms": f(&|e| st(e, "digest_ms")),
        "producers_ms": f(&|e| e["producers_ms"].as_f64().unwrap()),
        "overhead_ms (propagate, SQLite, observer)": f(&|e| e["overhead_ms"].as_f64().unwrap()),
        "total_ms": f(&|e| e["total_ms"].as_f64().unwrap()),
        "load_1m": f(&|e| e["load_1m"].as_f64().unwrap()),
    });
    let u = |e: &Json, a: &str, b: &str| e[a][b].as_f64().unwrap_or(0.0);
    let counts = json!({
        "waves": f(&|e| u(e, "work", "waves")),
        "rerun": f(&|e| u(e, "work", "rerun")),
        "changed": f(&|e| u(e, "work", "changed")),
        "ids_recomputed (rows)": f(&|e| e["step"]["rows"]["recomputed"].as_f64().unwrap()),
        "rows_changed": f(&|e| e["step"]["rows"]["rows_changed"].as_f64().unwrap()),
        "edges_deleted": f(&|e| e["step"]["rows"]["edges_del"].as_f64().unwrap()),
        "edges_inserted": f(&|e| e["step"]["rows"]["edges_ins"].as_f64().unwrap()),
        "ids_invalidated (values)": f(&|e| e["step"]["values"]["invalidated"].as_f64().unwrap()),
        "rounds (values)": f(&|e| e["step"]["values"]["rounds"].as_f64().unwrap()),
        "toggled": f(&|e| e["step"]["values"]["toggled"].as_f64().unwrap()),
        "findings_evaluated": f(&|e| st(e, "findings_evaluated")),
        "findings_changed": f(&|e| st(e, "findings_changed")),
    });
    let groups = |g: &[&str]| -> Json {
        let es: Vec<f64> = per_event.iter().filter(|e| g.contains(&e["group"].as_str().unwrap())).map(|e| e["total_ms"].as_f64().unwrap()).collect();
        summary(&es)
    };
    let res = json!({
        "mode": mode_label(mode), "graph": graph_kind, "events": per_event.len(), "single_file": single.len(),
        "seconds": {"world": world_s, "loaded": loaded_s, "events_wall": events_s, "events_cpu": events_cpu, "total_wall": t_all.elapsed().as_secs_f64(), "total_cpu": cols::cpu_seconds() - cpu0},
        "load_1m": {"start": load_start, "end": load_average()},
        "footprint_mb": {"end": footprint_mb(), "max": max_footprint_mb()},
        "base": base_info, "loads": loads, "memory": samples,
        "latency_single_file": latency, "counts_single_file": counts,
        "total_ms_by_group": {"bulk": groups(&["bulk"]), "commit": groups(&["commit", "commit_init"])},
        "exactness": {"checks": lines.len(), "forked": checks.forked, "unequal": unequal.len(), "first_unequal": unequal.iter().take(10).collect::<Vec<_>>(),
            "bindings_checked": bindings_checked, "bindings_unequal": parent_diffs.len(), "first_bindings_unequal": parent_diffs.iter().take(10).collect::<Vec<_>>(), "check_every": check_every, "check_load": check_load},
        "overlay": overlay.as_ref().map(|o| format!("{:?}", o.counters())),
    });
    std::fs::write(out.join("events.json"), serde_json::to_string(&per_event).unwrap()).unwrap();
    std::fs::write(out.join("results.json"), serde_json::to_string_pretty(&res).unwrap()).unwrap();
    println!("{}", serde_json::to_string_pretty(&json!({"out": out, "mode": res["mode"], "graph": res["graph"], "events": res["events"], "seconds": res["seconds"], "load_1m": res["load_1m"], "footprint_mb": res["footprint_mb"], "exactness": res["exactness"], "latency_single_file": res["latency_single_file"], "counts_single_file": res["counts_single_file"]})).unwrap());
    // Exit the replay's state without freeing it piece by piece (gigabytes of maps).
    std::mem::forget(dc);
    res
}

/// The per-edit benchmark: E3's replay through `propagate`, no checks.
#[test]
#[ignore = "per-edit benchmark on E3's replay; minutes and data outside the repository (see the module docs)"]
fn deadcode_replay_workload() {
    let res = run_big(|| replay("workload", 0, false));
    assert_eq!(res["exactness"]["unequal"], 0);
}

/// Exactness on E3's replay: after every load and every `RERAIL_DEADCODE_CHECK_EVERY` events
/// (default 1), in a forked child, `DeadCode::check` (rows = Y2's `succ`, the transpose, the
/// roots, levels = E1's fixpoint on the maintained CSR = Y2's reached set and levels from
/// scratch, findings = Y2's from scratch, every output digest = from scratch), and in the parent
/// the bound names = the producer's digests. Zero differences required.
#[test]
#[ignore = "exactness on E3's replay; minutes and data outside the repository (see the module docs)"]
fn deadcode_replay_exactness() {
    let res = run_big(|| replay("exactness", 1, true));
    assert!(res["exactness"]["checks"].as_u64().unwrap() > 0);
    assert_eq!(res["exactness"]["unequal"], 0, "{}", res["exactness"]);
    assert_eq!(res["exactness"]["bindings_unequal"], 0, "{}", res["exactness"]);
}

/// The oracle tree (`2b1504adc`, Y2's stage dump `dump-oracle`): dead code from the lattice
/// fact, both modes, through the derivation graph, = the dataflow's own `dc__findings` in the
/// dump = Y2 from scratch = 4,931 findings.
struct Oracle {
    top: Top,
}

impl Inputs for Oracle {
    fn top(&self, _: u32) -> &Top {
        &self.top
    }
    fn top_mut(&mut self, _: u32) -> &mut Top {
        &mut self.top
    }
    fn xref(&mut self, _: u32, _: &[&PathEdit]) -> wire::Xref {
        unreachable!("the oracle tree takes no edit")
    }
    fn symbols(&mut self, _: u32, _: &[&PathEdit]) -> (Vec<Change>, sym::Work) {
        unreachable!("the oracle tree takes no edit")
    }
    fn symbol_rows(&self, _: u32) -> Option<Vec<(Tab, Box<[Val]>)>> {
        None
    }
}

#[test]
#[ignore = "the oracle tree's 4,931 findings; a 447 MB dump outside the repository (see the module docs)"]
fn deadcode_oracle_tree_has_4931_findings() {
    let res = run_big(|| {
        let dump = path_env("RERAIL_DEADCODE_ORACLE_DUMP", "explore/y2-local/runs/20261023-210008-dump-oracle/dump.tsv");
        let out = run_dir("oracle");
        std::fs::create_dir_all(&out).unwrap();
        let t = Instant::now();
        let d = crate::code_rule_dsl::facts::y2::read_dump(&dump);
        let w = crate::code_rule_dsl::facts::y2::load_world(&d, None, d.reference_w.expect("a reference worktree"));
        let read_s = t.elapsed().as_secs_f64();
        let want = w.oracle.rows("dc__findings");
        let y2 = Dc::scratch(&w.facts).finding_rows();
        let mut results = Vec::new();
        let facts = w.facts;
        let (base, info) = Base::build(&facts, &out.join("base"));
        let refs = refs_digest(&facts);
        let rule = Rule::scratch(&facts);
        let top = Top { y1: Delta::default(), sym: SymState::default(), facts, rule: Some(rule), rev: LMap::default(), members: LMap::default(), refs_digest: refs, sym_digest: SetDigest::default() };
        let mut dc = DeadCode::new(Oracle { top }, base, Mode::Maintained);
        let mut digests = Vec::new();
        for mode in [Mode::Maintained, Mode::Recompute { threads: 1 }] {
            dc.mode = mode;
            let store = ContentStore::open(&out.join(format!("{}.sqlite", mode_label(mode)))).unwrap();
            dc.step = Step::default();
            let t = Instant::now();
            dc.create(&store, 1).unwrap();
            let got = dc.finding_rows(1);
            let check = dc.check(1);
            digests.push(store.binding(&Family::Findings.name(&scope_of(1))).unwrap());
            results.push(json!({"mode": mode_label(mode), "create_ms": t.elapsed().as_secs_f64() * 1e3, "findings": got.len(), "equal_dataflow": got == want, "equal_y2": got == y2, "check": check, "step": dc.step}));
        }
        let res = json!({"dump": dump, "read_s": read_s, "base": info, "dataflow_findings": want.len(), "y2_findings": y2.len(), "runs": results, "same_digest": digests[0] == digests[1], "footprint_mb": footprint_mb()});
        std::fs::write(out.join("results.json"), serde_json::to_string_pretty(&res).unwrap()).unwrap();
        println!("{}", serde_json::to_string_pretty(&res).unwrap());
        res
    });
    assert_eq!(res["dataflow_findings"], 4931);
    assert_eq!(res["y2_findings"], 4931);
    assert_eq!(res["same_digest"], true);
    for r in res["runs"].as_array().unwrap() {
        assert_eq!(r["findings"], 4931, "{r}");
        assert_eq!(r["equal_dataflow"], true, "{r}");
        assert_eq!(r["equal_y2"], true, "{r}");
        assert_eq!(r["check"], json!([]), "{r}");
    }
}
