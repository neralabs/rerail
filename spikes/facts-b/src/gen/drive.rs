//! `facts-b greplay`: the generated dataflow over the membership model, driven by Spike B's edit
//! replay. Every plan is an IR repository plan lowered by the generator; after every event the
//! edited worktree (and every worktree each 100 edits) is compared with the from-scratch
//! evaluation (the vectors: Spike B's oracle; IR plans with a reference: `rerail_facts::reference`).
use super::lower::{self, Out, Refusals, Source, T};
use super::norm;
use super::tags::{self, W, BASE};
use super::val::{self, Row};
use super::vectors;
use crate::derive::World;
use crate::membership::{self, Members, Resident};
use crate::replay::{read_events, rss, Event};
use crate::store::{self, Cid, Store, Sym};
use rustc_hash::FxHashMap;
use serde_json::json;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

pub struct Opts {
    pub events: Option<PathBuf>,
    pub limit: Option<usize>,
    pub oracle: bool,
    pub fail_fast: bool,
    pub clusters: bool,
    pub track: bool,
    pub plans: Vec<String>,
}

fn mb(b: u64) -> f64 {
    b as f64 / (1024.0 * 1024.0)
}

fn pct(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    s[((s.len() - 1) as f64 * p).round() as usize]
}

fn summary(v: &[f64]) -> serde_json::Value {
    json!({"n": v.len(), "p50_ms": pct(v, 0.5), "p95_ms": pct(v, 0.95), "p99_ms": pct(v, 0.99), "max_ms": pct(v, 1.0)})
}

fn show(r: &Row) -> String {
    let parts: Vec<String> = r.iter().map(|v| format!("{:?}", val::to_v(*v))).collect();
    format!("({})", parts.join(", "))
}

/// Exported rows per (export, path, content), computed once and held while some tag holds the
/// (path, content): eviction follows the membership model's resident counts.
struct Facts {
    world: Rc<RefCell<World>>,
    rows: FxHashMap<(u32, Sym, Cid), Rc<[Row]>>,
    exports: Vec<u32>,
}

impl Facts {
    fn get(&mut self, e: u32, p: Sym, c: Cid) -> Rc<[Row]> {
        if let Some(r) = self.rows.get(&(e, p, c)) {
            return r.clone();
        }
        let r: Rc<[Row]> = vectors::rows(&mut self.world.borrow_mut(), e, p, c).into();
        self.rows.insert((e, p, c), r.clone());
        r
    }
    fn weight(&mut self, p: Sym, c: Cid) -> u64 {
        let ex = self.exports.clone();
        1 + ex.iter().map(|e| self.get(*e, p, c).len() as u64).sum::<u64>()
    }
    fn evict(&mut self, p: Sym, c: Cid) {
        for e in self.exports.clone() {
            self.rows.remove(&(e, p, c));
        }
    }
    fn bytes(&self) -> usize {
        self.rows.values().map(|r| r.iter().map(|x| 24 + x.capacity() * std::mem::size_of::<val::Val>()).sum::<usize>() + 16).sum()
    }
}

pub fn run(run_dir: &Path, opts: Opts) {
    std::fs::create_dir_all(run_dir).unwrap();
    let t_load = Instant::now();
    let sd = store::store_dir();
    let mdir = store::spike_dir().join("membership");
    let script_base_m = store::read_membership(&mdir.join("base.tsv"));
    let ms = crate::base::load_memberships();
    let mut st = Store::default();
    for (_, h) in script_base_m.iter() {
        st.load(&sd, h);
    }
    for (_, m) in &ms {
        for (_, h) in m {
            st.load(&sd, h);
        }
    }
    let events: Vec<Event> = opts.events.as_ref().map(|p| read_events(p)).unwrap_or_default();
    let events: Vec<Event> = match opts.limit {
        Some(n) => events.into_iter().take(n).collect(),
        None => events,
    };
    let load_store_s = t_load.elapsed().as_secs_f64();
    let run_dir = run_dir.to_path_buf();

    timely::execute_directly(move |worker| {
        let world = Rc::new(RefCell::new(World::new(st)));
        // Plans: IR repository plans, normalised by the generator.
        let all: Vec<(&str, rerail_facts::ir::Plan<rerail_facts::ir::RepoSource>)> = vec![("r1", vectors::r1()), ("r3", vectors::r3())];
        let exported = vectors::exported();
        let mut graphs = Vec::new();
        let mut refusals_static = Vec::new();
        for (name, p) in all {
            if !opts.plans.is_empty() && !opts.plans.iter().any(|x| x == name) {
                continue;
            }
            match norm::plan(&p, &exported) {
                Ok(g) => graphs.push((name.to_string(), Rc::new(g))),
                Err(r) => refusals_static.push(json!({"plan": name, "refused": r.iter().map(|x| json!({"at": x.at, "construct": x.construct})).collect::<Vec<_>>()})),
            }
        }
        if !refusals_static.is_empty() {
            eprintln!("refused at compile time: {}", serde_json::to_string_pretty(&refusals_static).unwrap());
        }
        let exports: Vec<u32> = {
            let mut s = BTreeSet::new();
            for (_, g) in &graphs {
                for op in &g.nodes {
                    if let norm::Op::Exported(e) = op {
                        s.insert(*e);
                    }
                }
            }
            s.into_iter().collect()
        };
        let facts = Rc::new(RefCell::new(Facts { world: world.clone(), rows: FxHashMap::default(), exports }));
        let fs = facts.clone();
        let src: Source = Rc::new(move |e, p, c| fs.borrow_mut().get(e, p, c));

        // Memberships as (path sym -> content id).
        let to_map = |m: &store::Membership, wd: &mut World| -> Members { m.iter().map(|(p, hh)| (wd.path(p), wd.store.by_hash[hh])).collect() };
        let script_base_map = to_map(&script_base_m, &mut world.borrow_mut());
        let mut members: BTreeMap<W, Members> = BTreeMap::new();
        for (i, (_, m)) in ms.iter().enumerate() {
            members.insert(i as W + 1, to_map(m, &mut world.borrow_mut()));
        }
        // The membership model: per-path base, cluster bases chosen by an exact segmentation
        // over the worktrees in order, weighted by the exported rows each delta entry carries.
        let t_model = Instant::now();
        let order: Vec<(W, &Members)> = members.iter().map(|(w, m)| (*w, m)).collect();
        let weight = |p: Sym, c: Cid| facts.borrow_mut().weight(p, c);
        let mut model = if opts.clusters {
            membership::model(&order, &weight)
        } else {
            let all: Vec<&Members> = members.values().collect();
            let base = membership::plurality(&all);
            let parent = members.keys().map(|w| (*w, BASE)).collect();
            membership::Model { base, clusters: BTreeMap::new(), parent, cost: 0, flat_cost: 0 }
        };
        model.install();
        let model_s = t_model.elapsed().as_secs_f64();
        eprintln!("model: {} clusters, cost {} (flat {}), {:.2}s", model.clusters.len(), model.cost, model.flat_cost, model_s);
        // Nothing is resident until a record holds it.
        facts.borrow_mut().rows.clear();

        let probe = timely::dataflow::operators::probe::Handle::new();
        let mut built = lower::build(worker, graphs.clone(), src, &probe);
        let mut resident = Resident::default();
        let mut epoch: T = 0;
        let mut offer = |built: &mut lower::Built, resident: &mut Resident, ups: Vec<((W, (Sym, Cid)), i32)>, epoch: &mut T, worker: &mut timely::worker::Worker| {
            let (mut acq, mut rel) = (Vec::new(), Vec::new());
            for ((w, (p, c)), d) in &ups {
                resident.update(*w, *p, *c, *d, &mut acq, &mut rel);
            }
            for (r, d) in ups {
                built.files.update(r, d);
            }
            *epoch += 1;
            built.files.advance_to(*epoch);
            built.files.flush();
            worker.step_while(|| probe.less_than(epoch));
            rel
        };
        let t_build = Instant::now();
        let mut per_level_ms = Vec::new();
        for (t, recs) in model.level_records() {
            let t0 = Instant::now();
            let ups = recs.into_iter().map(|((p, c), d)| ((t, (p, c)), d)).collect();
            let rel = offer(&mut built, &mut resident, ups, &mut epoch, worker);
            for (p, c) in rel {
                facts.borrow_mut().evict(p, c);
            }
            per_level_ms.push(json!([t, t0.elapsed().as_secs_f64() * 1e3]));
        }
        let mut divergence = Vec::new();
        for (w, m) in &members {
            let d = membership::delta(m, model.below(*w));
            let paths: BTreeSet<Sym> = d.iter().map(|((p, _), _)| *p).collect();
            let rows: u64 = d.iter().map(|((p, c), _)| facts.borrow_mut().weight(*p, *c) - 1).sum();
            divergence.push(json!({"worktree": w, "parent": tags::parent(*w), "delta_paths": paths.len(), "delta_entries": d.len(), "delta_rows": rows}));
            let t0 = Instant::now();
            let ups = d.into_iter().map(|((p, c), s)| ((*w, (p, c)), s)).collect();
            let rel = offer(&mut built, &mut resident, ups, &mut epoch, worker);
            for (p, c) in rel {
                facts.borrow_mut().evict(p, c);
            }
            per_level_ms.push(json!([w, t0.elapsed().as_secs_f64() * 1e3]));
        }
        let load_s = t_build.elapsed().as_secs_f64();
        let rss_loaded = rss();
        eprintln!("loaded: {:.2}s, rss {:.1} MB, resident (path, content) {}", load_s, mb(rss_loaded.0), resident.held());

        // ---- checks
        let mut checks = 0usize;
        let mut worktree_checks = 0usize;
        let mut differences: Vec<serde_json::Value> = Vec::new();
        let mut oracle_s = 0.0;
        let mut check = |w: W, members: &BTreeMap<W, Members>, built: &lower::Built, at: usize, differences: &mut Vec<serde_json::Value>| {
            let t = Instant::now();
            let files: Vec<(Sym, Cid)> = members[&w].iter().map(|(p, c)| (*p, *c)).collect();
            let f = crate::oracle::evaluate(&mut world.borrow_mut(), &files);
            for (name, out) in &built.outputs {
                let plan = name.split(':').next().unwrap();
                let want: BTreeMap<Row, i32> = vectors::oracle_rows(&world.borrow(), &f, plan).into_iter().map(|r| (r, 1)).collect();
                match lower::full(out, w) {
                    Err(e) => differences.push(json!({"event": at, "worktree": w, "plan": name, "error": e})),
                    Ok(got) => {
                        if got != want {
                            let extra: Vec<String> = got.iter().filter(|(r, c)| want.get(*r) != Some(c)).take(5).map(|(r, c)| format!("{} x{c}", show(r))).collect();
                            let missing: Vec<String> = want.keys().filter(|r| !got.contains_key(*r)).take(5).map(show).collect();
                            differences.push(json!({"event": at, "worktree": w, "plan": name, "dataflow_only_or_multiplicity": extra, "oracle_only": missing, "dataflow": got.len(), "oracle": want.len()}));
                        }
                    }
                }
            }
            oracle_s += t.elapsed().as_secs_f64();
        };
        let mut initial = Vec::new();
        if opts.oracle {
            for w in members.keys().copied().collect::<Vec<_>>() {
                check(w, &members, &built, 0, &mut differences);
                worktree_checks += 1;
                initial.push(json!([w, built.outputs.iter().map(|(n, o)| (n.clone(), lower::full(o, w).map(|m| m.len()).unwrap_or(usize::MAX))).collect::<BTreeMap<_, _>>()]));
            }
            checks += 1;
            eprintln!("initial check: {} differences", differences.len());
        }

        // ---- replay
        let mut lat: BTreeMap<String, Vec<f64>> = BTreeMap::new();
        let mut per_event = Vec::new();
        let mut edits_done = 0usize;
        let mut rss_samples = vec![json!({"at": "loaded", "rss_mb": mb(rss_loaded.0), "footprint_mb": mb(rss_loaded.1)})];
        let mut fact_load_s = 0.0;
        let mut stopped = false;
        for ev in &events {
            if stopped {
                break;
            }
            let w = ev.worktree;
            let tl = Instant::now();
            let mut ups: Vec<((W, (Sym, Cid)), i32)> = Vec::new();
            {
                let mut wd = world.borrow_mut();
                if ev.group == "new_worktree" || ev.group == "commit_init" {
                    assert!(!members.contains_key(&w), "worktree {w} exists");
                    drop(wd);
                    let weight = |p: Sym, c: Cid| facts.borrow_mut().weight(p, c);
                    model.attach(w, &script_base_map, &weight);
                    for (rec, s) in membership::delta(&script_base_map, model.below(w)) {
                        ups.push(((w, rec), s));
                    }
                    members.insert(w, script_base_map.clone());
                    wd = world.borrow_mut();
                }
                let m = members.get_mut(&w).unwrap();
                for (path, old, new) in &ev.changes {
                    let p = wd.path(path);
                    let oldc = old.as_ref().map(|x| wd.store.load(&sd, &store::parse_hex(x)));
                    let newc = new.as_ref().map(|x| wd.store.load(&sd, &store::parse_hex(x)));
                    assert_eq!(m.get(&p).copied(), oldc, "event {} {path}: old content mismatch", ev.seq);
                    if let Some(c) = oldc {
                        ups.push(((w, (p, c)), -1));
                        m.remove(&p);
                    }
                    if let Some(c) = newc {
                        ups.push(((w, (p, c)), 1));
                        m.insert(p, c);
                    }
                }
            }
            // Exported rows of contents the event introduces are computed before the clock.
            for ((_, (p, c)), d) in &ups {
                if *d > 0 {
                    facts.borrow_mut().weight(*p, *c);
                }
            }
            fact_load_s += tl.elapsed().as_secs_f64();
            let t0 = Instant::now();
            let rel = offer(&mut built, &mut resident, ups, &mut epoch, worker);
            let ms_ = t0.elapsed().as_secs_f64() * 1e3;
            for (p, c) in rel {
                facts.borrow_mut().evict(p, c);
            }
            lat.entry(ev.group.clone()).or_default().push(ms_);
            if ev.group == "edit" {
                lat.entry(format!("edit:{}", ev.kind)).or_default().push(ms_);
                edits_done += 1;
            }
            per_event.push(json!([ev.seq, ev.group, ev.kind, w, ev.changes.len(), ms_]));
            if opts.oracle {
                let before = differences.len();
                check(w, &members, &built, ev.seq, &mut differences);
                worktree_checks += 1;
                checks += 1;
                if ev.group == "edit" && edits_done % 100 == 0 {
                    for ww in members.keys().copied().collect::<Vec<_>>() {
                        check(ww, &members, &built, ev.seq, &mut differences);
                        worktree_checks += 1;
                    }
                    eprintln!("edit {edits_done}: all-worktree check, {} differences so far", differences.len());
                }
                if (opts.fail_fast && differences.len() > before) || differences.len() > 50 {
                    eprintln!("stopping at event {}: {} differences", ev.seq, differences.len());
                    stopped = true;
                }
            }
            if ev.group == "edit" && edits_done % 100 == 0 {
                let r = rss();
                rss_samples.push(json!({"at": format!("edit {edits_done}"), "rss_mb": mb(r.0), "footprint_mb": mb(r.1), "resident_pairs": resident.held(), "resident_contents": resident.contents()}));
            }
        }
        let rss_end = rss();
        rss_samples.push(json!({"at": "end", "rss_mb": mb(rss_end.0), "footprint_mb": mb(rss_end.1)}));
        let mut lat_summary = serde_json::Map::new();
        for (k, v) in &lat {
            lat_summary.insert(k.clone(), summary(v));
        }
        let single: Vec<f64> = ["edit", "new_worktree_edit"].iter().flat_map(|k| lat.get(*k).cloned().unwrap_or_default()).collect();
        lat_summary.insert("single_file_all".into(), summary(&single));
        let Refusals(runtime_refusals) = lower::refusals();
        let (nsyms, sym_bytes) = val::syms_footprint();
        let out = json!({
            "plans": graphs.iter().map(|(n, g)| json!({"plan": n, "nodes": g.nodes.len()})).collect::<Vec<_>>(),
            "refused_static": refusals_static,
            "runtime_refusals": runtime_refusals,
            "events": events.len(),
            "model": {"clusters": model.clusters.iter().map(|(t, (ws, m))| json!({"tag": t, "worktrees": ws, "paths": m.len()})).collect::<Vec<_>>(), "cost": model.cost, "flat_cost": model.flat_cost, "seconds": model_s, "base_paths": model.base.len()},
            "load": {"store_seconds": load_store_s, "levels_and_deltas_seconds": load_s, "rss_loaded_mb": mb(rss_loaded.0), "per_level_ms": per_level_ms},
            "divergence_initial": divergence,
            "latency": lat_summary,
            "fact_load_seconds_total": fact_load_s,
            "rss": rss_samples,
            "resident": {"pairs": resident.held(), "contents": resident.contents(), "exported_row_bytes": facts.borrow().bytes(), "interned": nsyms, "interned_bytes": sym_bytes},
            "exactness": {"checks": checks, "worktree_checks": worktree_checks, "differences": differences.len(), "first_differences": differences.iter().take(20).collect::<Vec<_>>(), "oracle_seconds": oracle_s, "stopped_early": stopped},
            "initial_findings_per_worktree": initial,
        });
        std::fs::write(run_dir.join("results.json"), serde_json::to_string_pretty(&out).unwrap()).unwrap();
        std::fs::write(run_dir.join("events.json"), serde_json::to_string(&per_event).unwrap()).unwrap();
        println!("{}", serde_json::to_string_pretty(&json!({"latency": out["latency"], "rss": out["rss"], "exactness": out["exactness"], "load": out["load"]["levels_and_deltas_seconds"], "runtime_refusals": out["runtime_refusals"]})).unwrap());
        let _: &Out = &built.outputs[0].1;
    });
}
