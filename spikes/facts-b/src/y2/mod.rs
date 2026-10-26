//! Y2: dead code (`unreachable-internal-symbols`) and `exported-families` rewritten as local
//! queries over the reference graph and the symbol table (`facts`), with reachability the one
//! maintained property (`dc`), and the family windows keyed (`ef`).
//!
//! `facts-b y2 --dump FILE --out DIR [--scratch-every N]` reads a greplay stage dump
//! (`gen::y2dump`): per worktree, the tables at its first dump, then each event's row deltas. Per
//! event it runs both rules scoped to the event's changes, and every `--scratch-every` events from
//! scratch over the new state; it checks scoped = from scratch (findings and the reached set) and
//! both = the dataflow's own `dc__findings` / `ef__findings` in the dump, after every event.
pub mod dc;
pub mod ef;
pub mod facts;

use facts::{parse_row, Counts, Facts, Row, Tab, Val};
use serde_json::json;
use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::Path;
use std::time::Instant;

#[derive(Default)]
pub(crate) struct Event {
    pub seq: usize,
    pub w: u32,
    pub group: String,
    pub kind: String,
    pub paths: Vec<String>,
    pub rows: Vec<(usize, i32, Row)>,
    pub creates: bool,
    /// Y1's edge export: (graph ms, pairs added, pairs removed) of the event's step.
    graph: Option<(f64, usize, usize)>,
}

#[derive(Default)]
pub(crate) struct Dump {
    pub outputs: Vec<String>,
    pub tabs: Vec<Option<Tab>>,
    pub reference_w: Option<u32>,
    pub reference: Vec<(usize, i32, Row)>,
    pub init: BTreeMap<u32, Vec<(usize, i32, Row)>>,
    pub events: Vec<Event>,
}

pub(crate) fn read_dump(p: &Path) -> Dump {
    let f = std::io::BufReader::with_capacity(1 << 22, std::fs::File::open(p).unwrap());
    let mut d = Dump::default();
    for line in f.lines() {
        let line = line.unwrap();
        let mut it = line.splitn(5, '\t');
        match it.next().unwrap() {
            "O" => {
                let i: usize = it.next().unwrap().parse().unwrap();
                let name = it.next().unwrap().to_owned();
                assert_eq!(i, d.outputs.len());
                // An output listed twice (two plan nodes, one name): only the first is read.
                let tab = Tab::of_output(&name).filter(|t| !d.tabs.contains(&Some(*t)));
                d.tabs.push(tab);
                d.outputs.push(name);
            }
            t @ ("R" | "I") => {
                let w: u32 = it.next().unwrap().parse().unwrap();
                let k: usize = it.next().unwrap().parse().unwrap();
                let c: i32 = it.next().unwrap().parse().unwrap();
                let r = parse_row(it.next().unwrap());
                if t == "R" {
                    d.reference_w = Some(w);
                    d.reference.push((k, c, r));
                } else {
                    d.init.entry(w).or_default().push((k, c, r));
                    // I rows right after an event of `w` with no changes listed: `w` was created by it.
                    if let Some(e) = d.events.last_mut() {
                        if e.w == w && e.paths.is_empty() && e.rows.is_empty() {
                            e.creates = true;
                        }
                    }
                }
            }
            "E" => {
                let seq = it.next().unwrap().parse().unwrap();
                let w = it.next().unwrap().parse().unwrap();
                let group = it.next().unwrap().to_owned();
                let rest = it.next().unwrap_or("");
                let mut f = rest.split('\t');
                let kind = f.next().unwrap_or("").to_owned();
                let graph = match (f.next(), f.next(), f.next()) {
                    (Some(a), Some(b), Some(c)) => Some((a.parse().unwrap(), b.parse().unwrap(), c.parse().unwrap())),
                    _ => None,
                };
                let creates = graph.is_some() && (group == "new_worktree" || group == "commit_init");
                d.events.push(Event { seq, w, group, kind, creates, graph, ..Default::default() });
            }
            "P" => d.events.last_mut().unwrap().paths.push(it.next().unwrap().to_owned()),
            "D" => {
                let k: usize = it.next().unwrap().parse().unwrap();
                let c: i32 = it.next().unwrap().parse().unwrap();
                let r = parse_row(it.next().unwrap());
                d.events.last_mut().unwrap().rows.push((k, c, r));
            }
            o => panic!("dump line kind {o}"),
        }
    }
    d
}

/// The dataflow's outputs Y2 checks against (by stage name), as multisets.
#[derive(Default, Clone)]
struct Oracle {
    sets: BTreeMap<String, BTreeMap<Row, i32>>,
}

impl Oracle {
    fn update(&mut self, name: &str, r: &Row, c: i32) {
        let m = self.sets.entry(name.to_owned()).or_default();
        let e = m.entry(r.clone()).or_default();
        *e += c;
        if *e == 0 {
            m.remove(r);
        }
    }
    fn rows(&self, name: &str) -> Vec<Vec<Val>> {
        let mut v: Vec<Vec<Val>> = self.sets.get(name).map(|m| m.iter().flat_map(|(r, c)| std::iter::repeat_n(r.to_vec(), *c as usize)).collect()).unwrap_or_default();
        v.sort();
        v
    }
    fn strings(&self, name: &str) -> Vec<String> {
        let mut v: Vec<String> = self.sets.get(name).map(|m| m.keys().map(|r| facts::string(r[0].s())).collect()).unwrap_or_default();
        v.sort();
        v
    }
}

fn stage(name: &str) -> &str {
    name.rsplit_once("/stage/").map(|x| x.1).unwrap_or(name)
}

struct World {
    counts: Counts,
    facts: Facts,
    oracle: Oracle,
}

impl World {
    /// Load a snapshot (every row with its count).
    fn load(&mut self, rows: Vec<(Tab, Row, i32)>) {
        let ch = self.counts.classify(&rows);
        self.counts.apply(&rows);
        self.facts.apply(&ch);
    }
}

fn split(d: &Dump, rows: &[(usize, i32, Row)], oracle: &mut Oracle) -> Vec<(Tab, Row, i32)> {
    let mut out = Vec::new();
    for (k, c, r) in rows {
        match d.tabs[*k] {
            Some(t) => out.push((t, r.clone(), *c)),
            None => oracle.update(stage(&d.outputs[*k]), r, *c),
        }
    }
    out
}

fn diff_rows(a: &[Vec<Val>], b: &[Vec<Val>]) -> (Vec<String>, Vec<String>) {
    let (mut i, mut j) = (0, 0);
    let (mut only_a, mut only_b) = (Vec::new(), Vec::new());
    while i < a.len() || j < b.len() {
        if j == b.len() || (i < a.len() && a[i] < b[j]) {
            only_a.push(facts::show_row(&a[i]));
            i += 1;
        } else if i == a.len() || b[j] < a[i] {
            only_b.push(facts::show_row(&b[j]));
            j += 1;
        } else {
            i += 1;
            j += 1;
        }
    }
    (only_a, only_b)
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
    json!({"n": v.len(), "p50": pct(v, 0.5), "p95": pct(v, 0.95), "p99": pct(v, 0.99), "max": pct(v, 1.0), "mean": v.iter().sum::<f64>() / v.len().max(1) as f64})
}

/// Scoped = from scratch (findings and reached set), and both = the dataflow.
fn check(w: &World, dcs: &dc::Dc, efs: &ef::Ef, scratch: Option<(&dc::Dc, &ef::Ef)>, diffs: &mut Vec<serde_json::Value>, at: usize, wt: u32) -> bool {
    let mut ok = true;
    let mut cmp = |what: &str, a: Vec<Vec<Val>>, b: Vec<Vec<Val>>| {
        if a != b {
            let (x, y) = diff_rows(&a, &b);
            diffs.push(json!({"event": at, "worktree": wt, "what": what, "left": a.len(), "right": b.len(), "left_only": x.iter().take(5).collect::<Vec<_>>(), "right_only": y.iter().take(5).collect::<Vec<_>>()}));
            ok = false;
        }
    };
    cmp("dc scoped vs dataflow", dcs.finding_rows(), w.oracle.rows("dc__findings"));
    cmp("ef scoped vs dataflow", efs.finding_rows(), w.oracle.rows("ef__findings"));
    if let Some((d2, e2)) = scratch {
        cmp("dc scoped vs scratch", dcs.finding_rows(), d2.finding_rows());
        cmp("ef scoped vs scratch", efs.finding_rows(), e2.finding_rows());
        for l in [0u32, 1] {
            let (a, b) = (dcs.reached_strings(l), d2.reached_strings(l));
            if a != b {
                diffs.push(json!({"event": at, "worktree": wt, "what": format!("reached layer {l} scoped vs scratch"), "left": a.len(), "right": b.len()}));
                ok = false;
            }
        }
        if dcs.layer_counts() != d2.layer_counts() {
            diffs.push(json!({"event": at, "worktree": wt, "what": "reached dispatch states scoped vs scratch", "left": format!("{:?}", dcs.layer_counts()), "right": format!("{:?}", d2.layer_counts())}));
            ok = false;
        }
    }
    for (l, name) in [(0u32, "dc__reachable"), (1, "dc__annotation_reachable")] {
        if w.oracle.sets.contains_key(name) {
            let (a, b) = (dcs.reached_strings(l), w.oracle.strings(name));
            if a != b {
                let only_a: Vec<&String> = a.iter().filter(|x| b.binary_search(x).is_err()).take(5).collect();
                let only_b: Vec<&String> = b.iter().filter(|x| a.binary_search(x).is_err()).take(5).collect();
                diffs.push(json!({"event": at, "worktree": wt, "what": format!("{name} scoped vs dataflow"), "left": a.len(), "right": b.len(), "left_only": only_a, "right_only": only_b}));
                ok = false;
            }
        }
    }
    ok
}

/// The reference edges as (path, start, target path, target start), sorted.
fn edge_keys(f: &Facts) -> Vec<(u32, i64, u32, i64)> {
    let mut v: Vec<(u32, i64, u32, i64)> = f.refs_from.iter().flat_map(|(s, ts)| ts.iter().map(move |t| (s.0, s.1, t.0, t.1))).collect();
    v.sort_unstable();
    v
}

fn oracle_edge_keys(o: &Oracle) -> Vec<(u32, i64, u32, i64)> {
    let mut v: Vec<(u32, i64, u32, i64)> = o.sets.get("views/reference_edges").map(|m| m.keys().map(|r| (r[0].s(), r[1].i(), r[3].s(), r[4].i())).collect()).unwrap_or_default();
    v.sort_unstable();
    v
}

/// `--edges FILE`: the reference edges from Y1's maintained graph (`YGRAPH_Y2_EDGES`), the
/// dataflow's kept as the check.
pub fn run(dump: &Path, out: &Path, scratch_every: usize, edges: Option<&Path>) {
    std::fs::create_dir_all(out).unwrap();
    let t = Instant::now();
    let mut d = read_dump(dump);
    let y1 = edges.map(|p| {
        let e = read_dump(p);
        // Refs come from Y1 only: the dataflow's go to the oracle side.
        for (k, t) in d.tabs.iter_mut().enumerate() {
            if *t == Some(Tab::Refs) {
                *t = None;
                assert!(d.outputs[k].ends_with("views/reference_edges"));
            }
        }
        e
    });
    let y1_events: BTreeMap<(usize, u32), &Event> = y1.as_ref().map(|e| e.events.iter().map(|x| ((x.seq, x.w), x)).collect()).unwrap_or_default();
    let (mut edge_checks, mut edge_diffs) = (0usize, 0usize);
    let read_s = t.elapsed().as_secs_f64();
    eprintln!("y2: dump read in {read_s:.1}s: {} reference rows, {} worktrees initialised, {} events", d.reference.len(), d.init.len(), d.events.len());
    let mut worktrees: Vec<u32> = d.reference_w.into_iter().chain(d.init.keys().copied()).collect();
    worktrees.sort();
    worktrees.dedup();
    let mut per_event = Vec::new();
    let mut diffs = Vec::new();
    let mut lat: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut loads = Vec::new();
    let mut checks = 0usize;
    let mut base_reached: Option<rustc_hash::FxHashSet<dc::St>> = None;
    for &wt in &worktrees {
        // One worktree at a time: its events are independent of the others'.
        let t0 = Instant::now();
        let mut oracle = Oracle::default();
        let mut rows = split(&d, &d.reference, &mut oracle);
        if Some(wt) != d.reference_w {
            rows.extend(split(&d, d.init.get(&wt).map_or(&[][..], Vec::as_slice), &mut oracle));
        }
        if let Some(e) = &y1 {
            let refs = |rs: &[(usize, i32, Row)]| rs.iter().map(|(_, c, r)| (Tab::Refs, r.clone(), *c)).collect::<Vec<_>>();
            rows.extend(refs(&e.reference));
            if Some(wt) != e.reference_w {
                rows.extend(refs(e.init.get(&wt).map_or(&[][..], Vec::as_slice)));
            }
        }
        let mut world = World { counts: Counts::default(), facts: Facts::default(), oracle };
        world.load(rows);
        let load_ms = t0.elapsed().as_secs_f64() * 1e3;
        let t1 = Instant::now();
        let mut dcs = dc::Dc::scratch(&world.facts);
        let dc_ms = t1.elapsed().as_secs_f64() * 1e3;
        let t2 = Instant::now();
        let mut efs = ef::Ef::scratch(&world.facts);
        let ef_ms = t2.elapsed().as_secs_f64() * 1e3;
        let ok = check(&world, &dcs, &efs, None, &mut diffs, 0, wt);
        if wt == worktrees[0] {
            // `scripts/findings_parity.py` reads `parity_rows` in greplay's row format.
            let show = |r: &Vec<Val>| format!("({})", r.iter().map(|v| match v { Val::Null => "Null".to_owned(), Val::Int(i) => format!("Int({i})"), Val::S(s) => format!("Text({:?})", facts::string(*s)) }).collect::<Vec<_>>().join(", "));
            let edges: Vec<String> = world.facts.refs_from.iter().flat_map(|((p, s), ts)| ts.iter().map(move |(tp, t)| format!("(Text({:?}), Int({s}), Int(0), Text({:?}))", facts::string(*p), format!("{}#{t}", facts::string(*tp))))).collect();
            let pr = json!({"parity_rows": {"y2:dc": dcs.finding_rows().iter().map(show).collect::<Vec<_>>(), "y2:ef": efs.finding_rows().iter().map(show).collect::<Vec<_>>(), "y2:edges": edges}, "runtime_refusals_live": [], "refused_static": []});
            if let Some(m) = world.oracle.sets.get("ef__families") {
                let mut want: Vec<String> = m.keys().map(|r| facts::string(r[r.len() - 1].s())).collect();
                want.sort();
                let got = efs.families();
                eprintln!("y2: ef__families: {} selected, dataflow {}, equal {}", got.len(), want.len(), got == want);
                if got != want {
                    diffs.push(json!({"event": 0, "worktree": wt, "what": "ef families vs dataflow", "left": got.len(), "right": want.len()}));
                }
            }
            std::fs::create_dir_all(out.join("parity")).unwrap();
            std::fs::write(out.join("parity/results.json"), serde_json::to_string(&pr).unwrap()).unwrap();
        }
        checks += 1;
        let (sn, sb) = facts::footprint();
        let (rw, iw, mw, fw) = dcs.state_words();
        let lc = dcs.layer_counts();
        let ec = efs.counts();
        let mine = dcs.reached_set();
        let (only_w, only_base) = match &base_reached {
            None => {
                base_reached = Some(mine);
                (0, 0)
            }
            Some(b) => (mine.difference(b).count(), b.difference(&mine).count()),
        };
        loads.push(json!({"worktree": wt, "reached_vs_first": {"only_here": only_w, "only_first": only_base}, "dispatch_by_class": dcs.dispatch_by_class(&world.facts), "load_ms": load_ms, "dc_scratch_ms": dc_ms, "ef_scratch_ms": ef_ms, "exact": ok, "fact_rows": world.counts.rows(), "definitions": world.facts.defs.len(),
            "reference_edges": world.facts.refs_from.values().map(Vec::len).sum::<usize>(),
            "dc": {"findings": dcs.finding_rows().len(), "reached_flow": lc.0, "reached_annotated": lc.1, "reached_dispatch": lc.2, "state_words": {"reached": rw, "indexes": iw, "imported_modules": mw, "findings": fw}},
            "ef": {"findings": efs.finding_rows().len(), "surfaces": ec.0, "groups": ec.1, "eligible": ec.2, "state_words": efs.state_words()},
            "interner": {"strings": sn, "bytes": sb}}));
        eprintln!("y2: worktree {wt}: load {load_ms:.0} ms, dc {dc_ms:.0} ms ({} findings, reached {lc:?}), ef {ef_ms:.0} ms ({} findings), exact {ok}", dcs.finding_rows().len(), efs.finding_rows().len());
        let mut n = 0usize;
        for ev in d.events.iter().filter(|e| e.w == wt) {
            if ev.creates {
                continue;
            }
            n += 1;
            let mut rows = Vec::new();
            for (k, c, r) in &ev.rows {
                match d.tabs[*k] {
                    Some(t) => rows.push((t, r.clone(), *c)),
                    None => world.oracle.update(stage(&d.outputs[*k]), r, *c),
                }
            }
            let mut graph_ms = None;
            if y1.is_some() {
                let ye = y1_events.get(&(ev.seq, wt)).unwrap_or_else(|| panic!("event {} worktree {wt}: not in Y1's edge export", ev.seq));
                assert_eq!((ye.group.as_str(), ye.kind.as_str()), (ev.group.as_str(), ev.kind.as_str()));
                graph_ms = ye.graph.map(|g| g.0);
                rows.extend(ye.rows.iter().map(|(_, c, r)| (Tab::Refs, r.clone(), *c)));
            }
            let ta = Instant::now();
            let changes = world.counts.classify(&rows);
            let classify_ms = ta.elapsed().as_secs_f64() * 1e3;
            let pd = dcs.prepare(&world.facts, &changes);
            let pe = efs.prepare(&world.facts, &changes);
            let tb = Instant::now();
            world.counts.apply(&rows);
            world.facts.apply(&changes);
            let apply_ms = tb.elapsed().as_secs_f64() * 1e3;
            let sd = dcs.finish(&world.facts, pd);
            let se = efs.finish(&world.facts, pe);
            let dc_scoped = sd.prepare_ms + sd.finish_ms;
            let ef_scoped = se.prepare_ms + se.finish_ms;
            let scratch = scratch_every > 0 && n % scratch_every == 0;
            let (mut dcu, mut efu) = (None, None);
            if y1.is_some() {
                // Y1's edges = the dataflow's `views/reference_edges` after the event.
                edge_checks += 1;
                if edge_keys(&world.facts) != oracle_edge_keys(&world.oracle) {
                    edge_diffs += 1;
                    diffs.push(json!({"event": ev.seq, "worktree": wt, "what": "Y1 edges vs dataflow reference_edges"}));
                }
            }
            if let Some(g) = graph_ms {
                let single = ev.group == "edit" || ev.group == "new_worktree_edit";
                let gk = if single { "edit".to_owned() } else { ev.group.clone() };
                lat.entry(format!("{gk}:graph_y1")).or_default().push(g);
            }
            let ok = if scratch {
                let t3 = Instant::now();
                let d2 = dc::Dc::scratch(&world.facts);
                dcu = Some(t3.elapsed().as_secs_f64() * 1e3);
                let t4 = Instant::now();
                let e2 = ef::Ef::scratch(&world.facts);
                efu = Some(t4.elapsed().as_secs_f64() * 1e3);
                check(&world, &dcs, &efs, Some((&d2, &e2)), &mut diffs, ev.seq, wt)
            } else {
                check(&world, &dcs, &efs, None, &mut diffs, ev.seq, wt)
            };
            checks += 1;
            let single = ev.group == "edit" || ev.group == "new_worktree_edit";
            let g = if single { "edit".to_owned() } else { ev.group.clone() };
            lat.entry(format!("{g}:dc_scoped")).or_default().push(dc_scoped);
            lat.entry(format!("{g}:ef_scoped")).or_default().push(ef_scoped);
            lat.entry(format!("{g}:facts_apply")).or_default().push(classify_ms + apply_ms);
            if let Some(x) = dcu {
                lat.entry(format!("{g}:dc_unscoped")).or_default().push(x);
            }
            if let Some(x) = efu {
                lat.entry(format!("{g}:ef_unscoped")).or_default().push(x);
            }
            let mut tabs: BTreeMap<String, usize> = BTreeMap::new();
            for c in &changes {
                *tabs.entry(format!("{:?}", c.tab)).or_default() += 1;
            }
            per_event.push(json!({"graph_ms": graph_ms, "seq": ev.seq, "worktree": wt, "group": ev.group, "kind": ev.kind, "paths": ev.paths, "delta_rows": ev.rows.len(), "changes": changes.len(), "changes_by_table": tabs,
                "facts_ms": classify_ms + apply_ms, "dc_scoped_ms": dc_scoped, "ef_scoped_ms": ef_scoped, "dc_unscoped_ms": dcu, "ef_unscoped_ms": efu, "dc": sd, "ef": se, "exact": ok,
                "dc_findings": dcs.finding_rows().len(), "ef_findings": efs.finding_rows().len()}));
            if diffs.len() > 50 {
                break;
            }
        }
        eprintln!("y2: worktree {wt}: {n} events, {} differences so far", diffs.len());
        if diffs.len() > 50 {
            eprintln!("y2: stopping, too many differences");
            break;
        }
    }
    let mut ls = serde_json::Map::new();
    for (k, v) in &lat {
        ls.insert(k.clone(), summary(v));
    }
    let (sn, sb) = facts::footprint();
    let res = json!({"edges": edges.map(|p| p.display().to_string()), "edge_checks": {"checks": edge_checks, "differences": edge_diffs}, "dump": dump.display().to_string(), "read_seconds": read_s, "worktrees": worktrees.len(), "checks": checks, "differences": diffs.len(), "first_differences": diffs.iter().take(30).collect::<Vec<_>>(), "latency_ms": ls, "loads": loads, "interner": {"strings": sn, "bytes": sb}, "scratch_every": scratch_every});
    std::fs::write(out.join("results.json"), serde_json::to_string_pretty(&res).unwrap()).unwrap();
    std::fs::write(out.join("events.json"), serde_json::to_string(&per_event).unwrap()).unwrap();
    eprintln!("y2: {} checks, {} differences; latency {}", checks, diffs.len(), serde_json::to_string(&ls).unwrap());
}

/// `y2 --dump FILE --out DIR --fuzz N [--seed S]`: on the dump's first worktree, `N` random steps,
/// each removing (or putting back) 1–16 random rows across the tables, both rules scoped per step
/// and checked against scratch after each: exactness under deletions the replay rarely makes
/// (roots, base edges, definitions with dependants).
pub fn fuzz(dump: &Path, out: &Path, n: usize, seed: u64) {
    std::fs::create_dir_all(out).unwrap();
    let d = read_dump(dump);
    let mut oracle = Oracle::default();
    let rows = split(&d, &d.reference, &mut oracle);
    let tabs = [Tab::Refs, Tab::Bases, Tab::Roots, Tab::Public, Tab::Imports, Tab::Resolved, Tab::Ann, Tab::Hier, Tab::DeadTraits, Tab::ModuleNames, Tab::Dyn, Tab::Probe, Tab::EfTraits];
    let by_tab: Vec<Vec<(Row, i32)>> = tabs.iter().map(|t| rows.iter().filter(|r| r.0 == *t).map(|r| (r.1.clone(), r.2)).collect()).collect();
    let mut world = World { counts: Counts::default(), facts: Facts::default(), oracle };
    world.load(rows);
    let mut dcs = dc::Dc::scratch(&world.facts);
    let mut efs = ef::Ef::scratch(&world.facts);
    let mut removed: std::collections::BTreeSet<(usize, usize)> = Default::default();
    let mut s = seed;
    let mut rnd = |m: usize| {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((s >> 33) as usize) % m.max(1)
    };
    let mut diffs = Vec::new();
    let mut steps = Vec::new();
    for step in 0..n {
        let k = 1 + rnd(16);
        let mut delta = Vec::new();
        let mut picked = std::collections::BTreeSet::new();
        for _ in 0..k {
            let t = rnd(tabs.len());
            if by_tab[t].is_empty() {
                continue;
            }
            let i = rnd(by_tab[t].len());
            if !picked.insert((t, i)) {
                continue;
            }
            let (r, c) = &by_tab[t][i];
            if removed.remove(&(t, i)) {
                delta.push((tabs[t], r.clone(), *c));
            } else {
                removed.insert((t, i));
                delta.push((tabs[t], r.clone(), -*c));
            }
        }
        let changes = world.counts.classify(&delta);
        let pd = dcs.prepare(&world.facts, &changes);
        let pe = efs.prepare(&world.facts, &changes);
        world.counts.apply(&delta);
        world.facts.apply(&changes);
        let sd = dcs.finish(&world.facts, pd);
        let se = efs.finish(&world.facts, pe);
        let d2 = dc::Dc::scratch(&world.facts);
        let e2 = ef::Ef::scratch(&world.facts);
        let before = diffs.len();
        // The dataflow saw none of these steps: scoped is compared with scratch only.
        let mut probe = Vec::new();
        let ok = {
            let mut cmp = |what: &str, a: Vec<Vec<Val>>, b: Vec<Vec<Val>>| {
                if a != b {
                    let (x, y) = diff_rows(&a, &b);
                    probe.push(json!({"step": step, "what": what, "left_only": x.iter().take(5).collect::<Vec<_>>(), "right_only": y.iter().take(5).collect::<Vec<_>>()}));
                }
            };
            cmp("dc", dcs.finding_rows(), d2.finding_rows());
            cmp("ef", efs.finding_rows(), e2.finding_rows());
            for l in [0u32, 1] {
                if dcs.reached_strings(l) != d2.reached_strings(l) {
                    probe.push(json!({"step": step, "what": format!("reached layer {l}")}));
                }
            }
            if dcs.layer_counts() != d2.layer_counts() {
                probe.push(json!({"step": step, "what": "dispatch states", "scoped": format!("{:?}", dcs.layer_counts()), "scratch": format!("{:?}", d2.layer_counts())}));
            }
            probe.is_empty()
        };
        diffs.extend(probe);
        steps.push(json!({"step": step, "rows": delta.len(), "changes": changes.len(), "removed_total": removed.len(), "dc": sd, "ef": se, "exact": ok, "dc_findings": dcs.finding_rows().len(), "ef_findings": efs.finding_rows().len()}));
        if diffs.len() > before {
            eprintln!("y2 fuzz: step {step}: difference");
        }
        if diffs.len() > 20 {
            break;
        }
    }
    let res = json!({"dump": dump.display().to_string(), "steps": steps.len(), "seed": seed, "differences": diffs.len(), "first_differences": diffs.iter().take(20).collect::<Vec<_>>()});
    std::fs::write(out.join("fuzz.json"), serde_json::to_string_pretty(&res).unwrap()).unwrap();
    std::fs::write(out.join("fuzz-steps.json"), serde_json::to_string(&steps).unwrap()).unwrap();
    eprintln!("y2 fuzz: {} steps, {} differences", steps.len(), diffs.len());
}
