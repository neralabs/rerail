//! `facts-b y3 replay`: the wired path on S3's replay. Load: the base layer over the strict
//! majority membership, an overlay per cluster, the 40 worktrees as steps over their layer.
//! Every event is one `World::apply` (new worktrees: a step over the base). Exactness (with
//! `--check-every N`): in a forked child, the worktree from scratch — Y1's graph, the symbol
//! table, Y2's inputs, both rules, the M2 plans — against what was maintained. Measurements:
//! memory by component at load, every 100 events, at edit 1,000 and at the end; per-event
//! milliseconds and counts by component.
use super::m2::M2;
use super::sym::{PathChange, SymState};
use super::wire::{self, Shared};
use super::world::{below, World};
use super::{arg, bench, read_events};
use crate::y2::dc::Dc;
use crate::y2::ef::Ef;
use crate::y2::facts::{self, sort_changes, Change, Facts, Row, Tab};
use crate::ygraph::replay::{heap, plurality, CLUSTERS};
use crate::ygraph::world::{edge_rows, Below};
use crate::ygraph::{read_members, scratch as y1_scratch, shared as y1_shared};
use rustc_hash::FxHashSet;
use serde_json::json;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn mb(b: usize) -> f64 {
    b as f64 / 1e6
}

/// Size bucket of a single-file edit (bytes of the new content).
fn bucket(bytes: u64) -> &'static str {
    match bytes {
        0..=4095 => "<4KB",
        4096..=16383 => "4-16KB",
        16384..=65535 => "16-64KB",
        _ => ">=64KB",
    }
}

fn content_bytes(h: &str) -> u64 {
    std::fs::metadata(bench().join(format!("spike-b/contents/{h}.py"))).map_or(0, |m| m.len())
}

/// Memory by component (bytes), with the heap in use and the footprint.
pub fn memory(w: &World) -> serde_json::Value {
    let (h, f) = heap();
    let sum_layers = |key: &str, from: usize| -> usize { w.layers[from..].iter().map(|l| l.bytes()[key].as_u64().unwrap() as usize).sum() };
    let sum_tops = |key: &str| -> usize { w.wts.values().map(|t| t.top.bytes(false)[key].as_u64().unwrap() as usize).sum() };
    let mut comp = serde_json::Map::new();
    for key in ["sym", "y2_inputs", "dc", "ef", "rev", "members"] {
        comp.insert(key.into(), json!({"base": sum_layers(key, 0) - sum_layers(key, 1), "overlays": sum_layers(key, 1), "deltas": sum_tops(key)}));
    }
    comp.insert("y1".into(), json!({"base": w.layers[0].y1.total_bytes(), "overlays": w.layers[1..].iter().map(|l| l.y1.total_bytes()).sum::<usize>(), "deltas": w.wts.values().map(|t| t.top.y1.bytes()).sum::<usize>()}));
    let (y2n, y2b) = facts::footprint();
    let (gn, gb) = crate::gen::val::syms_footprint();
    json!({
        "heap_mb": mb(h as usize), "footprint_mb": mb(f as usize),
        "components": comp,
        "graph_ids": {"y1_queries": w.sh.y1.qt.bytes(), "y1_defs": w.sh.y1.defs.bytes(), "sym_queries": w.sh.sym.qt.bytes(), "sym_query_count": w.sh.sym.qt.len()},
        "per_file_inputs": {"y1_summaries": w.sh.y1.sums.bytes(), "sym_blocks": w.sh.sym.blocks.bytes(), "m2_exports": w.m2.bytes()},
        "strings": {"y1_interner": w.sh.y1.it.bytes(), "y1_strings": w.sh.y1.it.len(), "y2_interner_est": 2 * y2b + 48 * y2n, "y2_strings": y2n, "m2_interner": gb, "m2_strings": gn, "path_map": w.sh.y2_path.capacity() * 9},
        "worktrees": w.wts.len(),
        "findings_held": w.held_findings(),
    })
}

/// Numbers of `b` added into `a`, recursively (objects by key, arrays by position).
fn add_json(a: &mut serde_json::Value, b: &serde_json::Value) {
    match (a, b) {
        (serde_json::Value::Object(x), serde_json::Value::Object(y)) => {
            for (k, v) in y {
                add_json(x.entry(k.clone()).or_insert(json!(null)), v);
            }
        }
        (serde_json::Value::Array(x), serde_json::Value::Array(y)) => {
            x.resize(y.len().max(x.len()), json!(null));
            for (i, v) in y.iter().enumerate() {
                add_json(&mut x[i], v);
            }
        }
        (a @ serde_json::Value::Null, b) => *a = b.clone(),
        (a, b) if a.is_number() && b.is_number() => *a = json!(a.as_f64().unwrap() + b.as_f64().unwrap()),
        _ => {}
    }
}

/// Per index/table/query kind of every layered component: the base layer, Σ overlays, Σ deltas.
pub fn memory_detail(w: &World) -> serde_json::Value {
    let qt = &w.sh.sym.qt;
    let of = |t: &wire::Top, layer: bool| {
        json!({
            "sym": t.sym.by_kind(qt, layer),
            "y2_inputs": t.facts.by_index(layer),
            "dc": t.dc.as_ref().map(|d| d.by_map(layer)),
        })
    };
    let base = of(&w.layers[0].top, true);
    let mut overlays = json!(null);
    for l in &w.layers[1..] {
        add_json(&mut overlays, &of(&l.top, true));
    }
    let mut deltas = json!(null);
    for t in w.wts.values() {
        add_json(&mut deltas, &of(&t.top, false));
    }
    json!({"base": base, "overlays": overlays, "deltas": deltas})
}

/// The maintained worktree `w` against every component from scratch over its members.
fn check(world: &mut World, w: u32) -> serde_json::Value {
    let t = Instant::now();
    let World { sh, layers, wts, m2, .. } = world;
    let wt = &wts[&w];
    let members: Vec<(String, String)> = wt.top.members.iter().map(|(p, c)| (facts::string(*p), facts::string(*c))).collect();
    let mut diffs: Vec<String> = Vec::new();
    // Y1: edges as `linked_edges` rows, and the refusal counters
    let b = below(layers, wt.layer);
    let maintained = edge_rows(&sh.y1, &b, &wt.top.y1);
    let mem: Vec<(u32, u32)> = members.iter().map(|(p, h)| (sh.y1.it.intern(p), sh.y1.it.intern(h))).collect();
    let (sd, _) = y1_scratch(&mut sh.y1, &mem);
    let fresh = edge_rows(&sh.y1, &Below::default(), &sd);
    if maintained != fresh || wt.top.y1.counters != sd.counters {
        diffs.push(format!("y1: {} maintained / {} fresh edge rows, counters {:?} / {:?}", maintained.len(), fresh.len(), wt.top.y1.counters, sd.counters));
    }
    // the symbol table
    let mut st = SymState::default();
    let pc: Vec<PathChange> = members.iter().map(|(p, h)| PathChange { path: facts::sym(p), old: None, new: Some(facts::sym(h)) }).collect();
    st.apply(&mut sh.sym, &pc);
    let sym_m: FxHashSet<(Tab, Row)> = wt.top.sym.all_rows(&sh.sym).into_iter().collect();
    let sym_f: FxHashSet<(Tab, Row)> = st.all_rows(&sh.sym).into_iter().collect();
    if sym_m != sym_f || wt.top.sym.refused.len() != st.refused.len() {
        let ex: Vec<String> = sym_m.symmetric_difference(&sym_f).take(4).map(|x| format!("{:?} {}", x.0, facts::show_row(&x.1))).collect();
        diffs.push(format!("sym: {} maintained / {} fresh rows: {ex:?}", sym_m.len(), sym_f.len()));
    }
    // Y2's inputs from scratch: the symbol table's rows and the graph's edges
    let (_, mut rows) = wire::all_edges(sh, &Below::default(), &sd);
    let refs_f: FxHashSet<(u32, i64, u32, i64)> = rows.iter().map(|c| (c.row[0].s(), c.row[1].i(), c.row[3].s(), c.row[4].i())).collect();
    let refs_m: FxHashSet<(u32, i64, u32, i64)> = wt.top.facts.refs_from.iter().flat_map(|(s, ts)| ts.iter().map(move |t| (s.0, s.1, t.0, t.1))).collect();
    if refs_m != refs_f {
        diffs.push(format!("refs: {} maintained / {} fresh position pairs, {} differ", refs_m.len(), refs_f.len(), refs_m.symmetric_difference(&refs_f).count()));
    }
    rows.extend(st.all_rows(&sh.sym).into_iter().map(|(tab, row)| Change { tab, row, added: true }));
    sort_changes(&mut rows);
    let mut f = Facts::default();
    f.apply(&rows);
    let dc = Dc::scratch(&f);
    let ef = Ef::scratch(&f);
    let (dcm, efm) = (wt.top.dc.as_ref().unwrap(), wt.top.ef.as_ref().unwrap());
    let dc_n = dc.finding_rows().len();
    if dcm.finding_rows() != dc.finding_rows() {
        diffs.push(format!("{}: {} maintained / {} scratch findings", wire::DEAD_CODE, dcm.finding_rows().len(), dc_n));
    }
    if efm.finding_rows() != ef.finding_rows() {
        diffs.push(format!("{}: {} maintained / {} scratch findings", wire::FAMILIES, efm.finding_rows().len(), ef.finding_rows().len()));
    }
    if dcm.layer_counts() != dc.layer_counts() || dcm.reached_strings(0) != dc.reached_strings(0) || dcm.reached_strings(1) != dc.reached_strings(1) {
        diffs.push(format!("reached: {:?} maintained / {:?} scratch", dcm.layer_counts(), dc.layer_counts()));
    }
    // the M2 plans
    for p in m2.check(w) {
        diffs.push(format!("{p}: stored / batch from scratch differ"));
    }
    json!({"worktree": w, "equal": diffs.is_empty(), "differences": diffs, "dc_findings": dc_n, "ef_findings": ef.finding_rows().len(), "edges": fresh.len(), "check_s": t.elapsed().as_secs_f64()})
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
            if block && self.inflight < self.max {
                break;
            }
        }
    }

    /// `check(world, w)` in a forked child, appended to the checks file with `label`.
    fn spawn(&mut self, world: &mut World, w: u32, label: &str) {
        while self.inflight >= self.max {
            self.reap(true);
        }
        std::io::stdout().flush().ok();
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork");
        if pid == 0 {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(world, w)));
            let mut line = match r {
                Ok(j) => j,
                Err(_) => json!({"worktree": w, "equal": false, "differences": ["check panicked"]}),
            };
            line["at"] = json!(label);
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

fn pct(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    s[((s.len() - 1) as f64 * p).round() as usize]
}

fn summary(v: &[f64]) -> serde_json::Value {
    json!({"n": v.len(), "p50": pct(v, 0.5), "p95": pct(v, 0.95), "p99": pct(v, 0.99), "max": pct(v, 1.0), "mean": v.iter().sum::<f64>() / v.len().max(1) as f64})
}

pub fn run(args: &[String]) {
    let out = PathBuf::from(arg(args, "--out").expect("--out DIR"));
    std::fs::create_dir_all(&out).unwrap();
    let b = bench();
    let limit: Option<usize> = arg(args, "--limit").map(|x| x.parse().unwrap());
    let check_every: usize = arg(args, "--check-every").map_or(0, |x| x.parse().unwrap());
    let check_load = args.iter().any(|a| a == "--check-load");
    let workers: usize = arg(args, "--workers").map_or(8, |x| x.parse().unwrap());
    let exports = PathBuf::from(arg(args, "--exports").unwrap_or_else(|| b.join("explore/x6-rules/exports/replay-symbols-merged.jsonl").display().to_string()));
    let spec = PathBuf::from(arg(args, "--spec").unwrap_or_else(|| b.join("explore/y3-wire/specs/spec-replay-m2-three.json").display().to_string()));
    let events_p = PathBuf::from(arg(args, "--events").unwrap_or_else(|| b.join("spike-b/edits/seed-5eedb/events.jsonl").display().to_string()));
    let mdir = b.join("spike-b/membership");
    let mut checks = Checks { file: out.join("checks.jsonl"), inflight: 0, max: workers, forked: 0 };
    let _ = std::fs::remove_file(&checks.file);
    let mut samples: Vec<serde_json::Value> = Vec::new();
    let t_all = Instant::now();

    // inputs: the per-content exports (Y1's summaries and the symbol table's blocks, one file),
    // M2's plans and their export index
    let mut sh = Shared { y1: y1_shared(), sym: Default::default(), y2_path: Default::default() };
    let mem0 = heap();
    let t = Instant::now();
    let n_y1 = crate::ygraph::summary::load(&exports, &mut sh.y1.it, &mut sh.y1.sums, None);
    let y1_load_s = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let n_sym = super::input::load(&exports, &mut sh.sym.blocks, None);
    let sym_load_s = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let m2 = M2::new(&spec);
    let m2_load_s = t.elapsed().as_secs_f64();
    let mem_inputs = heap();
    eprintln!("y3: inputs: {n_y1} summaries ({y1_load_s:.1}s), {n_sym} blocks ({sym_load_s:.1}s), M2 {} plans ({m2_load_s:.1}s); heap {:.0} MB", m2.plans.len(), mb(mem_inputs.0 as usize));

    // layers
    let base_m = read_members(&mdir.join("base.tsv"));
    let t = Instant::now();
    let (base, base_info) = wire::base_layer(&mut sh, &base_m);
    let base_s = t.elapsed().as_secs_f64();
    eprintln!("y3: base layer {base_s:.1}s: {}", serde_json::to_string(&base_info).unwrap());
    let mut world = World { sh, layers: vec![base], m2, wts: BTreeMap::new(), names: BTreeMap::new() };
    let ms: Vec<Vec<(String, String)>> = (1..=40u32).map(|w| read_members(&mdir.join(format!("{w:02}.tsv")))).collect();
    let t = Instant::now();
    let mut cluster_info = Vec::new();
    for &(lo, hi) in &CLUSTERS {
        let cm = plurality(&ms, lo, hi);
        let World { sh, layers, .. } = &mut world;
        let b = below(layers, 0);
        let (top, st) = wire::over(sh, &b, &layers[0], &cm);
        let layer = top.freeze(true);
        cluster_info.push(json!({"worktrees": [lo, hi], "divergence": st.paths, "ms": st.ms, "bytes": layer.bytes()}));
        world.layers.push(layer);
    }
    let clusters_s = t.elapsed().as_secs_f64();
    eprintln!("y3: {} overlays in {clusters_s:.1}s", CLUSTERS.len());
    let layer_of = |w: u32| CLUSTERS.iter().position(|&(lo, hi)| lo <= w && w <= hi).map_or(0, |c| c + 1);
    let t = Instant::now();
    let mut loads = Vec::new();
    for w in 1..=40u32 {
        let st = world.create(w, layer_of(w), &ms[w as usize - 1]);
        loads.push(json!({"worktree": w, "layer": layer_of(w), "divergence": st.paths, "ms": st.ms, "y1_ms": st.y1_ms, "refs_ms": st.refs_ms, "sym_ms": st.sym.ms, "y2_ms": st.y2_ms, "m2_ms": st.m2_ms,
            "bytes": world.wts[&w].top.bytes(false), "y1_bytes": world.wts[&w].top.y1.bytes(), "step": st}));
        if check_load {
            checks.spawn(&mut world, w, &format!("load w{w:02}"));
        }
    }
    let loads_s = t.elapsed().as_secs_f64();
    let mut m = memory(&world);
    m["at"] = json!("loaded");
    m["detail"] = memory_detail(&world);
    eprintln!("y3: 40 worktrees in {loads_s:.1}s; {}", serde_json::to_string(&m).unwrap());
    samples.push(m);

    // events
    let events = read_events(&events_p);
    let events: Vec<_> = events.into_iter().take(limit.unwrap_or(usize::MAX)).collect();
    let mut per_event = Vec::new();
    let mut edits = 0usize;
    let mut refusals_seen = 0u64;
    let mut floor: Vec<f64> = Vec::new();
    for (i, ev) in events.iter().enumerate() {
        let created = ev.group == "new_worktree" || ev.group == "commit_init";
        let mut create = None;
        if created {
            assert!(!world.wts.contains_key(&ev.worktree));
            let st = world.create(ev.worktree, 0, &base_m);
            create = Some(json!({"ms": st.ms, "y1_ms": st.y1_ms, "sym_ms": st.sym.ms, "y2_ms": st.y2_ms, "m2_ms": st.m2_ms, "m2": st.m2}));
        }
        let st = world.apply(ev.worktree, &ev.changes);
        refusals_seen += st.sym.refused;
        let single = ev.group == "edit" || ev.group == "new_worktree_edit";
        let bytes = if single { ev.changes[0].2.as_deref().map_or(0, content_bytes) } else { 0 };
        let wt = &world.wts[&ev.worktree];
        per_event.push(json!({
            "seq": ev.seq, "worktree": ev.worktree, "group": ev.group, "kind": ev.kind, "paths": st.paths, "bytes": bytes, "bucket": if single { bucket(bytes) } else { "" },
            "ms": st.ms, "y1_ms": st.y1_ms, "refs_ms": st.refs_ms, "sym_ms": st.sym.ms, "y2_ms": st.y2_ms, "m2_ms": st.m2_ms, "create": create,
            "y1": {"rows_touched": st.y1.table_rows_touched, "keys_changed": st.y1.keys_changed, "work": st.y1.work, "edges_added": st.y1.edges_added.len(), "edges_removed": st.y1.edges_removed.len(), "paths_reblocked": st.y1.paths_reblocked, "out_paths": st.y1.out_paths, "init_fallback": st.y1.init_fallback},
            "refs_changes": st.refs_changes, "sym": st.sym, "sym_changes": st.sym_changes, "dc": st.dc, "ef": st.ef, "m2": st.m2,
            "findings_delta": st.delta.len(), "delta_bytes": wt.top.bytes(false), "y1_delta_bytes": wt.top.y1.bytes(),
            "failures": world.failures(ev.worktree).len(),
        }));
        if ev.group == "edit" {
            edits += 1;
        }
        // the floor: the same event re-applied (every content unchanged) costs only the dispatch
        if single && i % 10 == 0 {
            let same: Vec<(String, Option<String>, Option<String>)> = ev.changes.iter().map(|(p, _, n)| (p.clone(), n.clone(), n.clone())).collect();
            let t = Instant::now();
            let st = world.apply(ev.worktree, &same);
            assert!(st.delta.is_empty() && st.paths == 0);
            floor.push(t.elapsed().as_secs_f64() * 1e3);
        }
        if check_every > 0 && (i + 1) % check_every == 0 {
            checks.spawn(&mut world, ev.worktree, &format!("event {}", ev.seq));
        }
        let at_1000 = ev.group == "edit" && edits == 1000;
        if (i + 1) % 100 == 0 || at_1000 {
            let mut m = memory(&world);
            m["at"] = json!(if at_1000 { "edit 1000".to_owned() } else { format!("event {}", i + 1) });
            if at_1000 {
                m["detail"] = memory_detail(&world);
            }
            m["events"] = json!(i + 1);
            eprintln!("y3: event {}: heap {:.0} MB, checks forked {}", i + 1, m["heap_mb"].as_f64().unwrap(), checks.forked);
            samples.push(m);
        }
    }
    checks.finish();
    let mut m = memory(&world);
    m["at"] = json!("end");
    m["detail"] = memory_detail(&world);
    samples.push(m);
    // exactness
    let lines: Vec<serde_json::Value> = std::fs::read_to_string(&checks.file).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let unequal: Vec<&serde_json::Value> = lines.iter().filter(|l| !l["equal"].as_bool().unwrap()).collect();
    let live: BTreeMap<u32, Vec<String>> = world.wts.keys().map(|w| (*w, world.failures(*w))).filter(|(_, f)| !f.is_empty()).collect();
    let res = json!({
        "inputs": {"exports": exports.display().to_string(), "spec": spec.display().to_string(), "y1_summaries": n_y1, "y1_load_s": y1_load_s, "sym_blocks": n_sym, "sym_load_s": sym_load_s, "m2_load_s": m2_load_s,
                   "heap_before_mb": mb(mem0.0 as usize), "heap_inputs_mb": mb(mem_inputs.0 as usize), "m2_plans": world.m2.plan_names()},
        "base": base_info, "base_s": base_s, "clusters": cluster_info, "clusters_s": clusters_s, "loads": loads, "loads_s": loads_s,
        "memory": samples, "floor_ms": summary(&floor), "events": per_event.len(), "seconds": t_all.elapsed().as_secs_f64(),
        "exactness": {"checks": lines.len(), "forked": checks.forked, "unequal": unequal.len(), "first_unequal": unequal.iter().take(10).collect::<Vec<_>>(), "check_every": check_every, "check_load": check_load},
        "runtime_refusals": {"seen": refusals_seen, "live_at_end": live, "m2_raised": crate::gen::lower::refusals().0},
        "anomalies": world.sh.y1.anomalies,
    });
    std::fs::write(out.join("events.json"), serde_json::to_string(&per_event).unwrap()).unwrap();
    std::fs::write(out.join("results.json"), serde_json::to_string_pretty(&res).unwrap()).unwrap();
    let mut short = res.clone();
    short.as_object_mut().unwrap().remove("loads");
    println!("{}", serde_json::to_string_pretty(&short).unwrap());
    if !unequal.is_empty() {
        std::process::exit(1);
    }
}

/// The spec file is the P3a light spec minus `exported-families` (Y2 runs it).
/// A rule file that moved with its worktree is read from this checkout's `packs/` (the same
/// file: the packs are tracked).
pub fn write_spec(p: &Path) {
    let light: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(bench().join("phase-2/s3-crossfile/exports/w6/spec-replay-light.json")).unwrap()).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let three: Vec<serde_json::Value> = light
        .into_iter()
        .filter(|x| x["clauses"][0] != "exported-families")
        .map(|mut x| {
            let rule = x["rule"].as_str().unwrap().to_owned();
            if !Path::new(&rule).exists() {
                let i = rule.find("/packs/").unwrap_or_else(|| panic!("{rule}: missing"));
                x["rule"] = json!(root.join(&rule[i + 1..]).display().to_string());
            }
            x
        })
        .collect();
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, serde_json::to_string_pretty(&three).unwrap()).unwrap();
}
