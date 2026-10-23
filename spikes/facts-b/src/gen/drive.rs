//! `facts-b greplay`: the generated dataflow over the membership model, driven by Spike B's edit
//! replay. Every plan is an IR repository plan lowered by the generator: the test vectors
//! (Spike B's R1/R3 as IR, `vectors`) and real plans compiled by `rerail_facts::compile`
//! (`real`). After every `--check-every`-th event the edited worktree (and every 100 edits the
//! `--sweep` worktrees) is compared with a from-scratch evaluation: the vectors with Spike B's
//! oracle, real plans with a second dataflow over the same plans whose only records are the
//! worktree's members, tagged base, run to completion and dropped (`scratch`); with
//! `--scratch reference`, with the reference evaluator
//! (`rerail_facts::reference::evaluate_repository`) over the worktree's member exported rows.
//! Findings and runtime refusals must both agree.
use super::lower::{self, Refusals, Source, T};
use super::norm;
use super::real::{self, Real};
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
    /// Test vectors to run (`r1`, `r3`); `none` for none.
    pub plans: Vec<String>,
    /// Real plans: spec JSON (`gen::real::load`).
    pub real: Option<PathBuf>,
    /// One worktree from a jobs file (path, hash, source) instead of the 40: parity runs.
    pub parity: Option<PathBuf>,
    /// Real plans checked by the reference evaluator instead of a from-scratch dataflow.
    pub reference_scratch: bool,
    /// The edited worktree is checked after every n-th event.
    pub check_every: usize,
    /// Worktrees checked at load and every 100 edits (evenly spaced); `None`: all.
    pub sweep: Option<usize>,
}

/// A plan's from-scratch view of one worktree: its runtime refusals when any (findings are then
/// withheld), else its findings.
enum Want {
    Rows(BTreeMap<Row, i32>),
    /// Spike B's oracle or the reference saw a tie the plan's order must refuse.
    Tie(String),
    /// The from-scratch dataflow's live refusals.
    Refused(Vec<String>),
    Error(String),
}

/// Real plans from scratch for one worktree: a second dataflow over `graphs` fed `files` as base
/// records only (every product is base, presence a threshold), run to completion, read and
/// dropped. Per output its findings; per plan its live refusals. Refusals it raises are not
/// counted in the run's.
#[allow(clippy::type_complexity)]
fn scratch(worker: &mut timely::worker::Worker, graphs: &[(String, Rc<norm::Graph>)], src: &Source, files: &[(Sym, Cid)]) -> (BTreeMap<String, Result<BTreeMap<Row, i32>, String>>, BTreeMap<String, Result<Vec<String>, String>>) {
    lower::uncounted(|| {
        let probe = timely::dataflow::operators::probe::Handle::new();
        let id = worker.next_dataflow_index();
        let mut b = lower::build(worker, graphs.to_vec(), src.clone(), &probe, false);
        for (p, c) in files {
            b.files.update((BASE, (*p, *c)), 1);
        }
        b.files.advance_to(1);
        b.files.flush();
        worker.step_while(|| probe.less_than(&1));
        let outs = b.outputs.iter().map(|(n, o)| (n.clone(), lower::full(o, BASE))).collect();
        let refs = b.refusals.iter().map(|(n, o)| (n.clone(), lower::full(o, BASE).map(|m| m.keys().map(show).collect()))).collect();
        drop(b);
        worker.drop_dataflow(id);
        (outs, refs)
    })
}

/// `k` worktrees evenly spaced over `ws` (all of them when `k` is `None` or covers them).
fn spread(ws: &[W], k: Option<usize>) -> Vec<W> {
    match k {
        Some(k) if k < ws.len() => (0..k).map(|i| ws[i * ws.len() / k]).collect(),
        _ => ws.to_vec(),
    }
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

/// Exported rows per (export, path, content), computed (vectors) or read (real plans) when a tag
/// first holds the (path, content), evicted when none does.
struct Facts {
    world: Rc<RefCell<World>>,
    real: Option<Rc<Real>>,
    rows: FxHashMap<(u32, Sym, Cid), Rc<[Row]>>,
    /// Exported row count per (path, content), for the membership model's weights.
    counts: FxHashMap<(Sym, Cid), u64>,
    exports: Vec<u32>,
    /// Keep computed rows until eviction (`FACTS_ROW_CACHE=0`: recompute on every record).
    cache: bool,
}

impl Facts {
    fn compute(&self, e: u32, p: Sym, c: Cid) -> Vec<Row> {
        if e < real::STRIDE {
            return vectors::rows(&mut self.world.borrow_mut(), e, p, c);
        }
        let (path, hash) = {
            let w = self.world.borrow();
            (w.store.syms.str(p).to_owned(), crate::facts::hex(&w.store.contents[c as usize].hash))
        };
        match self.real.as_ref().unwrap().rows(e, &path, &hash) {
            Ok(r) => r,
            Err(m) => {
                lower::refuse_external(&m);
                Vec::new()
            }
        }
    }
    fn get(&mut self, e: u32, p: Sym, c: Cid) -> Rc<[Row]> {
        if let Some(r) = self.rows.get(&(e, p, c)) {
            return r.clone();
        }
        let r: Rc<[Row]> = self.compute(e, p, c).into();
        if self.cache {
            self.rows.insert((e, p, c), r.clone());
        }
        r
    }
    /// Make a (path, content)'s rows resident before its records enter (not timed).
    fn prefetch(&mut self, p: Sym, c: Cid) {
        for e in self.exports.clone() {
            self.get(e, p, c);
        }
    }
    fn weight(&mut self, p: Sym, c: Cid) -> u64 {
        if let Some(n) = self.counts.get(&(p, c)) {
            return *n;
        }
        let mut n = 1;
        for e in self.exports.clone() {
            n += match self.rows.get(&(e, p, c)) {
                Some(r) => r.len() as u64,
                None => self.compute(e, p, c).len() as u64,
            };
        }
        self.counts.insert((p, c), n);
        n
    }
    fn evict(&mut self, p: Sym, c: Cid) {
        for e in self.exports.clone() {
            self.rows.remove(&(e, p, c));
        }
    }
    fn bytes(&self) -> usize {
        self.rows.values().map(|r| r.iter().map(|x| std::mem::size_of::<Row>() + x.heap_bytes()).sum::<usize>() + 16).sum()
    }
}

fn read_jobs(p: &Path) -> store::Membership {
    let mut m: store::Membership = std::fs::read_to_string(p)
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let mut it = l.split('\t');
            let path = it.next().unwrap().to_owned();
            (path, store::parse_hex(it.next().unwrap()))
        })
        .collect();
    m.sort();
    m
}

pub fn run(run_dir: &Path, opts: Opts) {
    std::fs::create_dir_all(run_dir).unwrap();
    let t_load = Instant::now();
    let sd = store::store_dir();
    let mdir = store::spike_dir().join("membership");
    let vector_names: Vec<String> = if opts.plans.is_empty() && opts.real.is_none() { vec!["r1".into(), "r3".into()] } else { opts.plans.iter().filter(|p| *p != "none").cloned().collect() };
    let script_base_m = store::read_membership(&mdir.join("base.tsv"));
    let ms: Vec<(String, store::Membership)> = match &opts.parity {
        Some(j) => vec![("parity".into(), read_jobs(j))],
        None => crate::base::load_memberships(),
    };
    // v0 facts only for the vectors (Spike B's derivations read them); real plans read their
    // producers' exports, contents need ids only.
    let mut st = Store { ids_only: vector_names.is_empty(), ..Default::default() };
    for (_, h) in script_base_m.iter() {
        st.load(&sd, h);
    }
    for (_, m) in &ms {
        for (_, h) in m {
            st.load(&sd, h);
        }
    }
    let events: Vec<Event> = if opts.parity.is_some() { Vec::new() } else { opts.events.as_ref().map(|p| read_events(p)).unwrap_or_default() };
    let events: Vec<Event> = match opts.limit {
        Some(n) => events.into_iter().take(n).collect(),
        None => events,
    };
    let load_store_s = t_load.elapsed().as_secs_f64();
    let run_dir = run_dir.to_path_buf();

    timely::execute_directly(move |worker| {
        let world = Rc::new(RefCell::new(World::new(st)));
        // One string table: the store's interner holds every text value of the dataflow.
        val::install(world.clone());
        // Plans: IR repository plans, normalised by the generator.
        let mut graphs = Vec::new();
        let mut refusals_static = Vec::new();
        let vexp = vectors::exported();
        for (name, p) in [("r1", vectors::r1()), ("r3", vectors::r3()), ("rt", vectors::rt())] {
            if !vector_names.iter().any(|x| x == name) {
                continue;
            }
            match norm::plan(&p, &vexp) {
                Ok(g) => graphs.push((name.to_string(), Rc::new(g))),
                Err(r) => refusals_static.push(json!({"plan": name, "refused": r.iter().map(|x| json!({"at": x.at, "construct": x.construct})).collect::<Vec<_>>()})),
            }
        }
        let real: Option<Rc<Real>> = opts.real.as_ref().map(|p| Rc::new(real::load(p)));
        if let Some(r) = &real {
            graphs.extend(r.graphs.iter().cloned());
            refusals_static.extend(r.refused.iter().cloned());
        }
        if !refusals_static.is_empty() {
            eprintln!("refused at compile time: {}", serde_json::to_string(&refusals_static).unwrap());
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
        let rss_store = rss();
        let cache = std::env::var("FACTS_ROW_CACHE").map_or(true, |v| v != "0");
        let facts = Rc::new(RefCell::new(Facts { world: world.clone(), real: real.clone(), rows: FxHashMap::default(), counts: FxHashMap::default(), exports, cache }));
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
        let mut model = if opts.clusters && members.len() > 1 {
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

        let probe = timely::dataflow::operators::probe::Handle::new();
        let sizes = crate::flow::register_logging(worker);
        let t_dataflow = Instant::now();
        let mut built = lower::build(worker, graphs.clone(), src.clone(), &probe, opts.track);
        let dataflow_s = t_dataflow.elapsed().as_secs_f64();
        eprintln!("dataflow built: {:.2}s, {} plan nodes", dataflow_s, graphs.iter().map(|(_, g)| g.nodes.len()).sum::<usize>());
        let mut resident = Resident::default();
        let mut epoch: T = 0;
        let prof_dir = run_dir.clone();
        let prof_sizes = sizes.clone();
        let offer = |built: &mut lower::Built, resident: &mut Resident, ups: Vec<((W, (Sym, Cid)), i32)>, epoch: &mut T, worker: &mut timely::worker::Worker| {
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
            if super::prof::on() {
                while probe.less_than(epoch) {
                    worker.step();
                    super::prof::tick(&prof_dir, &prof_sizes.borrow(), false);
                }
                super::prof::tick(&prof_dir, &prof_sizes.borrow(), true);
            } else {
                worker.step_while(|| probe.less_than(epoch));
            }
            rel
        };
        let t_build = Instant::now();
        let mut per_level_ms = Vec::new();
        for (t, recs) in model.level_records() {
            let t0 = Instant::now();
            for ((p, c), d) in &recs {
                if *d > 0 {
                    facts.borrow_mut().prefetch(*p, *c);
                }
            }
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
            for ((p, c), s) in &d {
                if *s > 0 {
                    facts.borrow_mut().prefetch(*p, *c);
                }
            }
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

        // ---- checks: the dataflow's view of worktree w against a from-scratch evaluation.
        let mut checks = 0usize;
        let mut worktree_checks = 0usize;
        let mut differences: Vec<serde_json::Value> = Vec::new();
        let mut oracle_s = 0.0;
        // Runtime refusals checked against the from-scratch side: per plan, (worktree checks
        // where a refusal was expected, of those refused); expected but not refused; refused
        // without an expectation (a difference); and the live refusals seen.
        let mut expected_refusals: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        let mut missing_expected_refusals = 0usize;
        let mut live_seen: BTreeMap<String, usize> = BTreeMap::new();
        let plan_of: FxHashMap<String, usize> = real.as_ref().map(|r| r.plans.iter().enumerate().map(|(i, (n, ..))| (n.clone(), i)).collect()).unwrap_or_default();
        let reference_scratch = opts.reference_scratch;
        // From-scratch dataflows built for checks, and the seconds they took (in `oracle_s`).
        let mut scratch_runs = 0usize;
        let mut scratch_s = 0.0;
        let mut check = |worker: &mut timely::worker::Worker, w: W, members: &BTreeMap<W, Members>, built: &lower::Built, at: usize, differences: &mut Vec<serde_json::Value>| {
            let t = Instant::now();
            let files: Vec<(Sym, Cid)> = members[&w].iter().map(|(p, c)| (*p, *c)).collect();
            let vf = if vector_names.iter().any(|v| v == "r1" || v == "r3") { Some(crate::oracle::evaluate(&mut world.borrow_mut(), &files)) } else { None };
            // Member files in path order (the IR's union order for exported relations).
            let mut ordered: Vec<(String, Sym, Cid)> = files.iter().map(|(p, c)| (world.borrow().store.syms.str(*p).to_owned(), *p, *c)).collect();
            ordered.sort();
            // Real plans from scratch: one second dataflow per check, over every real plan.
            let fresh = match &real {
                Some(r) if !reference_scratch => {
                    let ts = Instant::now();
                    let x = scratch(worker, &r.graphs, &src, &files);
                    scratch_runs += 1;
                    scratch_s += ts.elapsed().as_secs_f64();
                    Some(x)
                }
                _ => None,
            };
            for (name, out) in &built.outputs {
                let plan = name.split(':').next().unwrap();
                // The plan's live runtime refusals in w.
                let live: Vec<String> = match built.refusals.iter().find(|(n, _)| n == plan).map(|(_, o)| lower::full(o, w)) {
                    Some(Ok(m)) => m.keys().map(|r| show(r)).collect(),
                    Some(Err(e)) => {
                        differences.push(json!({"event": at, "worktree": w, "plan": name, "refusal_error": e}));
                        continue;
                    }
                    None => Vec::new(),
                };
                for l in &live {
                    *live_seen.entry(format!("{plan}: {l}")).or_default() += 1;
                }
                let rows = |r: Result<BTreeMap<Row, i32>, String>| r.map_or_else(Want::Error, Want::Rows);
                let want = if let (Some(_), Some((outs, refs))) = (plan_of.get(plan), &fresh) {
                    match refs.get(plan).cloned().unwrap_or_else(|| Ok(Vec::new())) {
                        Err(e) => Want::Error(format!("scratch refusals: {e}")),
                        Ok(x) if !x.is_empty() => Want::Refused(x),
                        Ok(_) => rows(outs.get(name).cloned().unwrap_or_else(|| Err(format!("scratch has no output {name}")))),
                    }
                } else if let Some(&i) = plan_of.get(plan) {
                    let r = real.as_ref().unwrap();
                    let (_, k, p, _) = &r.plans[i];
                    let exported = |id: u32| -> Result<Vec<Vec<rerail_facts::sql::value::V>>, String> {
                        let e = (*k as u32 + 1) * real::STRIDE + id;
                        let mut rows = Vec::new();
                        for (_, ps, c) in &ordered {
                            rows.extend(facts.borrow_mut().compute(e, *ps, *c).iter().map(|x| val::row_to_v(x)));
                        }
                        Ok(rows)
                    };
                    match rerail_facts::reference::evaluate_repository(p, &exported) {
                        Ok(rs) => {
                            let mut m = BTreeMap::new();
                            for r in rs {
                                *m.entry(r.iter().map(val::from_v).collect::<Row>()).or_default() += 1;
                            }
                            Want::Rows(m)
                        }
                        Err(e) if e.starts_with("tie") => Want::Tie(e),
                        Err(e) => Want::Error(e),
                    }
                } else if plan == "rt" {
                    match vectors::rt_oracle(&mut world.borrow_mut(), &files) {
                        (true, _) => Want::Tie("tie: row_number() order with ties between distinct rows".into()),
                        (false, rows) => Want::Rows(rows.into_iter().map(|r| (r, 1)).collect()),
                    }
                } else {
                    Want::Rows(vectors::oracle_rows(vf.as_ref().unwrap(), plan).into_iter().map(|r| (r, 1)).collect())
                };
                let want = match want {
                    // A refusal expected from scratch: the incremental side must hold it (the
                    // same constructs, when the from-scratch side is a dataflow).
                    Want::Tie(_) | Want::Refused(_) => {
                        let e = expected_refusals.entry(plan.to_owned()).or_default();
                        e.0 += 1;
                        if live.is_empty() {
                            missing_expected_refusals += 1;
                        }
                        match &want {
                            Want::Refused(x) if *x != live => differences.push(json!({"event": at, "worktree": w, "plan": name, "refusals_differ": {"incremental": live, "scratch": x}})),
                            Want::Tie(x) if live.is_empty() => differences.push(json!({"event": at, "worktree": w, "plan": name, "missing_expected_refusal": x})),
                            _ => e.1 += 1,
                        }
                        continue;
                    }
                    Want::Error(e) => {
                        differences.push(json!({"event": at, "worktree": w, "plan": name, "reference_error": e}));
                        continue;
                    }
                    Want::Rows(x) => x,
                };
                if !live.is_empty() {
                    differences.push(json!({"event": at, "worktree": w, "plan": name, "unexpected_runtime_refusal": live}));
                    continue;
                }
                match lower::full(out, w) {
                    Err(e) => differences.push(json!({"event": at, "worktree": w, "plan": name, "error": e})),
                    Ok(got) => {
                        if got != want {
                            let extra: Vec<String> = got.iter().filter(|(r, c)| want.get(*r) != Some(c)).take(5).map(|(r, c)| format!("{} x{c}", show(r))).collect();
                            let missing: Vec<String> = want.iter().filter(|(r, c)| got.get(*r) != Some(c)).take(5).map(|(r, c)| format!("{} x{c}", show(r))).collect();
                            differences.push(json!({"event": at, "worktree": w, "plan": name, "dataflow_only": extra, "reference_only": missing, "dataflow": got.len(), "reference": want.len()}));
                        }
                    }
                }
            }
            oracle_s += t.elapsed().as_secs_f64();
        };
        let sweep = |members: &BTreeMap<W, Members>| spread(&members.keys().copied().collect::<Vec<_>>(), opts.sweep);
        let mut initial = Vec::new();
        if opts.oracle {
            for w in sweep(&members) {
                check(worker, w, &members, &built, 0, &mut differences);
                worktree_checks += 1;
                initial.push(json!([w, built.outputs.iter().map(|(n, o)| (n.clone(), lower::full(o, w).map(|m| m.values().map(|c| *c as usize).sum::<usize>()).unwrap_or(usize::MAX))).collect::<BTreeMap<_, _>>()]));
            }
            checks += 1;
            eprintln!("initial check: {} differences", differences.len());
        }
        // Parity runs: the findings rows of the one worktree.
        let parity_rows: Option<BTreeMap<String, Vec<String>>> = opts.parity.as_ref().map(|_| built.outputs.iter().map(|(n, o)| (n.clone(), lower::full(o, 1).unwrap_or_default().into_iter().flat_map(|(r, c)| std::iter::repeat_n(show(&r), c as usize)).collect())).collect());

        // ---- replay
        let mut lat: BTreeMap<String, Vec<f64>> = BTreeMap::new();
        let mut per_event = Vec::new();
        let mut edits_done = 0usize;
        let mut rss_samples = vec![json!({"at": "loaded", "rss_mb": mb(rss_loaded.0), "footprint_mb": mb(rss_loaded.1)})];
        let mut fact_load_s = 0.0;
        let mut stopped = false;
        let mut events_done = 0usize;
        for ev in &events {
            if stopped {
                break;
            }
            let w = ev.worktree;
            let tl = Instant::now();
            let mut ups: Vec<((W, (Sym, Cid)), i32)> = Vec::new();
            if ev.group == "new_worktree" || ev.group == "commit_init" {
                assert!(!members.contains_key(&w), "worktree {w} exists");
                let weight = |p: Sym, c: Cid| facts.borrow_mut().weight(p, c);
                model.attach(w, &script_base_map, &weight);
                for (rec, s) in membership::delta(&script_base_map, model.below(w)) {
                    ups.push(((w, rec), s));
                }
                members.insert(w, script_base_map.clone());
            }
            {
                let mut wd = world.borrow_mut();
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
            // Exported rows of contents the event introduces are made resident before the clock.
            for ((_, (p, c)), d) in &ups {
                if *d > 0 {
                    facts.borrow_mut().prefetch(*p, *c);
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
            events_done += 1;
            if opts.oracle {
                let before = differences.len();
                if events_done % opts.check_every == 0 {
                    check(worker, w, &members, &built, ev.seq, &mut differences);
                    worktree_checks += 1;
                    checks += 1;
                }
                if ev.group == "edit" && edits_done % 100 == 0 {
                    for ww in sweep(&members) {
                        check(worker, ww, &members, &built, ev.seq, &mut differences);
                        worktree_checks += 1;
                    }
                    eprintln!("edit {edits_done}: worktree sweep, {} differences so far", differences.len());
                }
                if (opts.fail_fast && differences.len() > before) || differences.len() > 50 {
                    eprintln!("stopping at event {}: {} differences", ev.seq, differences.len());
                    stopped = true;
                }
            }
            if ev.group == "edit" && edits_done % 100 == 0 {
                let r = rss();
                let arranged: isize = sizes.borrow().records.values().sum();
                rss_samples.push(json!({"at": format!("edit {edits_done}"), "rss_mb": mb(r.0), "footprint_mb": mb(r.1), "resident_pairs": resident.held(), "resident_contents": resident.contents(), "arranged": arranged}));
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
        drop(check);
        let Refusals(runtime_refusals) = lower::refusals();
        // Live runtime refusals at the end, per plan and worktree.
        let runtime_refusals_live: Vec<serde_json::Value> = built
            .refusals
            .iter()
            .flat_map(|(plan, o)| members.keys().filter_map(move |w| lower::full(o, *w).ok().filter(|m| !m.is_empty()).map(|m| json!({"plan": plan, "worktree": w, "refusals": m.keys().map(|r| show(r)).collect::<Vec<_>>()}))))
            .collect();
        let (nsyms, sym_bytes) = val::syms_footprint();
        // Per-worktree state (with --track): live records per tag over every operator output.
        let state = if opts.track {
            let per = lower::tracked();
            let mut rows = Vec::new();
            for (w, m) in &members {
                let d = membership::delta(m, model.below(*w));
                let drows: u64 = d.iter().map(|((p, c), _)| facts.borrow_mut().weight(*p, *c) - 1).sum();
                let (recs, bytes) = per.get(w).copied().unwrap_or((0, 0));
                rows.push(json!({"worktree": w, "parent": tags::parent(*w), "delta_entries": d.len(), "delta_rows": drows, "records": recs, "bytes": bytes}));
            }
            for (t, (_, cb)) in &model.clusters {
                let d = membership::delta(cb, &model.base);
                let (recs, bytes) = per.get(t).copied().unwrap_or((0, 0));
                rows.push(json!({"cluster": t, "delta_entries": d.len(), "records": recs, "bytes": bytes}));
            }
            let (recs, bytes) = per.get(&BASE).copied().unwrap_or((0, 0));
            rows.push(json!({"base": 0, "records": recs, "bytes": bytes}));
            json!(rows)
        } else {
            json!(null)
        };
        // Arrangement records from differential's logging, per operator name (`n<node>:<role>`).
        let arrangements = {
            let sz = sizes.borrow();
            let mut by_name: BTreeMap<String, (isize, isize)> = BTreeMap::new();
            for (op, n) in &sz.records {
                by_name.entry(sz.names.get(op).cloned().unwrap_or_else(|| format!("op{op}"))).or_default().0 += n;
            }
            for (op, b) in &sz.batcher_bytes {
                by_name.entry(sz.names.get(op).cloned().unwrap_or_else(|| format!("op{op}"))).or_default().1 += b;
            }
            let total: isize = by_name.values().map(|v| v.0).sum();
            let mut arr: Vec<(String, (isize, isize))> = by_name.into_iter().filter(|(_, v)| v.0 != 0 || v.1 != 0).collect();
            arr.sort_by_key(|x| -x.1 .0);
            json!({"total_records": total, "by_operator": arr.iter().map(|(n, (r, b))| json!([n, r, b])).collect::<Vec<_>>()})
        };
        let out = json!({
            "arrangements": arrangements,
            "construction": lower::construction(),
            "plans": graphs.iter().map(|(n, g)| json!({"plan": n, "nodes": g.nodes.len()})).collect::<Vec<_>>(),
            "refused_static": refusals_static,
            "runtime_refusals": runtime_refusals,
            "runtime_refusals_live": runtime_refusals_live,
            "runtime_refusals_seen": live_seen,
            "expected_refusals": expected_refusals.iter().map(|(p, (e, r))| (p.clone(), json!({"expected": e, "refused": r}))).collect::<BTreeMap<_, _>>(),
            "missing_expected_refusals": missing_expected_refusals,
            "events": events.len(),
            "model": {"clusters": model.clusters.iter().map(|(t, (ws, m))| json!({"tag": t, "worktrees": ws, "paths": m.len()})).collect::<Vec<_>>(), "cost": model.cost, "flat_cost": model.flat_cost, "seconds": model_s, "base_paths": model.base.len()},
            "load": {"store_seconds": load_store_s, "dataflow_build_seconds": dataflow_s, "levels_and_deltas_seconds": load_s, "rss_store_mb": mb(rss_store.0), "rss_loaded_mb": mb(rss_loaded.0), "per_level_ms": per_level_ms},
            "divergence_initial": divergence,
            "latency": lat_summary,
            "fact_load_seconds_total": fact_load_s,
            "rss": rss_samples,
            "resident": {"pairs": resident.held(), "contents": resident.contents(), "exported_row_bytes": facts.borrow().bytes(), "interned": nsyms, "interned_bytes": sym_bytes},
            "exactness": {"checks": checks, "worktree_checks": worktree_checks, "differences": differences.len(), "first_differences": differences.iter().take(20).collect::<Vec<_>>(), "oracle_seconds": oracle_s, "stopped_early": stopped, "scratch": if opts.reference_scratch { "reference" } else { "engine" }, "scratch_dataflows": scratch_runs, "scratch_seconds": scratch_s, "check_every": opts.check_every, "sweep": opts.sweep},
            "initial_findings_per_worktree": initial,
            "state": state,
            "parity_rows": parity_rows,
        });
        std::fs::write(run_dir.join("results.json"), serde_json::to_string_pretty(&out).unwrap()).unwrap();
        std::fs::write(run_dir.join("events.json"), serde_json::to_string(&per_event).unwrap()).unwrap();
        println!("{}", serde_json::to_string_pretty(&json!({"latency": out["latency"]["single_file_all"], "rss": out["rss"], "exactness": out["exactness"], "load": out["load"]["levels_and_deltas_seconds"], "runtime_refusals": out["runtime_refusals"], "refused_static": out["refused_static"]})).unwrap());
    });
}
