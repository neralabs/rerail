//! Steady-state driver (steps 5 and 6): start from facts on disk, load the base and 40 worktree
//! deltas into one dataflow, replay the edit script, time each event (input accepted -> output
//! frontier advanced), and compare against the from-scratch oracle.
use crate::base;
use crate::derive::World;
use crate::flow::{self, Handles, Out, W};
use crate::oracle;
use crate::store::{self, Cid, Store, Sym};
use serde::Deserialize;
use serde_json::json;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::Hash;
use std::path::Path;
use std::rc::Rc;
use std::time::Instant;

#[derive(Deserialize, Clone, Debug)]
pub struct Event {
    pub seq: usize,
    /// edit | bulk | new_worktree | new_worktree_edit | commit_init | commit
    pub group: String,
    pub kind: String,
    pub worktree: W,
    /// (path, old hash, new hash); None = absent.
    pub changes: Vec<(String, Option<String>, Option<String>)>,
}

pub fn read_events(p: &Path) -> Vec<Event> {
    std::fs::read_to_string(p).unwrap().lines().filter(|l| !l.is_empty()).map(|l| serde_json::from_str(l).unwrap()).collect()
}

pub fn rss() -> (u64, u64) {
    let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
    let r = unsafe { libc::proc_pid_rusage(libc::getpid(), libc::RUSAGE_INFO_V2, &mut info as *mut _ as *mut libc::rusage_info_t) };
    assert_eq!(r, 0);
    (info.ri_resident_size, info.ri_phys_footprint)
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
    json!({"n": v.len(), "p50_ms": pct(v, 0.5), "p95_ms": pct(v, 0.95), "p99_ms": pct(v, 0.99), "max_ms": pct(v, 1.0),
           "mean_ms": if v.is_empty() { f64::NAN } else { v.iter().sum::<f64>() / v.len() as f64 }})
}

/// Full findings of worktree w from the dataflow: base output + w's delta. Err on a
/// multiplicity other than 1 (the result must be a set).
fn full<F: Clone + Ord + Hash + Eq>(out: &Out<F>, w: W) -> Result<BTreeSet<F>, String> {
    let o = out.borrow();
    let mut m: BTreeMap<F, flow::R> = BTreeMap::new();
    for ((ww, f), d) in o.iter() {
        if *ww == flow::BASE || *ww == w {
            *m.entry(f.clone()).or_default() += d;
        }
    }
    let mut s = BTreeSet::new();
    for (f, c) in m {
        match c {
            0 => {}
            1 => {
                s.insert(f);
            }
            _ => return Err(format!("multiplicity {c}")),
        }
    }
    Ok(s)
}

struct Check {
    checks: usize,
    worktree_checks: usize,
    differences: Vec<serde_json::Value>,
    oracle_seconds: f64,
}

fn diff_sets<F: Ord + std::fmt::Debug>(a: &BTreeSet<F>, b: &BTreeSet<F>) -> (Vec<String>, Vec<String>) {
    (a.difference(b).take(5).map(|x| format!("{x:?}")).collect(), b.difference(a).take(5).map(|x| format!("{x:?}")).collect())
}

pub struct Opts {
    pub oracle: bool,
    pub events: Option<std::path::PathBuf>,
    pub limit: Option<usize>,
    pub track: bool,
}

pub fn run(run_dir: &Path, opts: Opts) {
    std::fs::create_dir_all(run_dir).unwrap();
    let t_load = Instant::now();
    let sd = store::store_dir();
    let mdir = store::spike_dir().join("membership");
    let base_m = store::read_membership(&mdir.join("base.tsv"));
    let ms = base::load_memberships();
    let mut st = Store::default();
    for (_, h) in &base_m {
        st.load(&sd, h);
    }
    for (_, m) in &ms {
        for (_, h) in m {
            st.load(&sd, h);
        }
    }
    let events = opts.events.as_ref().map(|p| read_events(p)).unwrap_or_default();
    let events: Vec<Event> = match opts.limit {
        Some(n) => events.into_iter().take(n).collect(),
        None => events,
    };
    let load_store_s = t_load.elapsed().as_secs_f64();
    let rss_after_store = rss();
    let oracle_on = opts.oracle;
    let track_on = opts.track;
    let run_dir = run_dir.to_path_buf();

    timely::execute_directly(move |worker| {
        let world = Rc::new(RefCell::new(World::new(st)));
        let t_build = Instant::now();
        let sizes = flow::register_logging(worker);
        let mut h: Handles = flow::build(worker, world.clone(), track_on);
        // Memberships as (path sym -> content id).
        let to_map = |m: &store::Membership, wd: &mut World| -> BTreeMap<Sym, Cid> {
            m.iter().map(|(p, hh)| (wd.path(p), wd.store.by_hash[hh])).collect()
        };
        let base_map = to_map(&base_m, &mut world.borrow_mut());
        let mut members: BTreeMap<W, BTreeMap<Sym, Cid>> = BTreeMap::new();
        for (i, (_, m)) in ms.iter().enumerate() {
            members.insert(i as W + 1, to_map(m, &mut world.borrow_mut()));
        }
        let mut divergence: BTreeMap<W, (usize, usize)> = BTreeMap::new();
        let delta_of = |m: &BTreeMap<Sym, Cid>| -> Vec<((Sym, Cid), flow::R)> {
            let mut v = Vec::new();
            for (p, c) in m {
                if base_map.get(p) != Some(c) {
                    v.push(((*p, *c), 1));
                }
            }
            for (p, c) in &base_map {
                if m.get(p) != Some(c) {
                    v.push(((*p, *c), -1));
                }
            }
            v
        };
        let mut epoch: flow::T = 0;
        // Base first, then all deltas, as two timed epochs.
        for (p, c) in &base_map {
            h.files.update((flow::BASE, (*p, *c)), 1);
        }
        epoch += 1;
        h.files.advance_to(epoch);
        h.files.flush();
        worker.step_while(|| h.probe.less_than(&epoch));
        let base_s = t_build.elapsed().as_secs_f64();
        let rss_after_base = rss();
        let t_d = Instant::now();
        let mut per_wt_load_ms = Vec::new();
        for (w, m) in &members {
            let d = delta_of(m);
            let paths: BTreeSet<Sym> = d.iter().map(|((p, _), _)| *p).collect();
            divergence.insert(*w, (paths.len(), d.len()));
            let t = Instant::now();
            for (rec, s) in d {
                h.files.update((*w, rec), s);
            }
            epoch += 1;
            h.files.advance_to(epoch);
            h.files.flush();
            worker.step_while(|| h.probe.less_than(&epoch));
            per_wt_load_ms.push(json!([w, paths.len(), t.elapsed().as_secs_f64() * 1e3]));
        }
        let deltas_s = t_d.elapsed().as_secs_f64();
        let rss_loaded = rss();
        eprintln!("loaded: base {:.2}s, 40 deltas {:.2}s, rss {:.1} MB footprint {:.1} MB", base_s, deltas_s, mb(rss_loaded.0), mb(rss_loaded.1));
        let nbase = |o: &dyn Fn() -> usize| o();
        let base_counts = (
            nbase(&|| h.r1.borrow().keys().filter(|k| k.0 == flow::BASE).count()),
            nbase(&|| h.r2.borrow().keys().filter(|k| k.0 == flow::BASE).count()),
            nbase(&|| h.r3.borrow().keys().filter(|k| k.0 == flow::BASE).count()),
        );

        let mut check = Check { checks: 0, worktree_checks: 0, differences: Vec::new(), oracle_seconds: 0.0 };
        let mut check_wt = |w: W, members: &BTreeMap<W, BTreeMap<Sym, Cid>>, h: &Handles, check: &mut Check, at: usize| {
            let t = Instant::now();
            let files: Vec<(Sym, Cid)> = members[&w].iter().map(|(p, c)| (*p, *c)).collect();
            let f = oracle::evaluate(&mut world.borrow_mut(), &files);
            check.worktree_checks += 1;
            for (rule, res) in [
                ("R1", full(&h.r1, w).map(|d| diff_sets(&d, &f.r1))),
                ("R2", full(&h.r2, w).map(|d| diff_sets(&d, &f.r2))),
                ("R3", full(&h.r3, w).map(|d| diff_sets(&d, &f.r3))),
            ] {
                match res {
                    Ok((a, b)) if a.is_empty() && b.is_empty() => {}
                    Ok((a, b)) => check.differences.push(json!({"event": at, "worktree": w, "rule": rule, "incremental_only": a, "oracle_only": b})),
                    Err(e) => check.differences.push(json!({"event": at, "worktree": w, "rule": rule, "error": e})),
                }
            }
            check.oracle_seconds += t.elapsed().as_secs_f64();
        };
        let mut sizes0 = BTreeMap::new();
        if oracle_on {
            for w in members.keys().copied().collect::<Vec<_>>() {
                check_wt(w, &members, &h, &mut check, 0);
                let f = full(&h.r1, w).map(|s| s.len()).unwrap_or(0);
                let f2 = full(&h.r2, w).map(|s| s.len()).unwrap_or(0);
                let f3 = full(&h.r3, w).map(|s| s.len()).unwrap_or(0);
                sizes0.insert(w, (f, f2, f3));
            }
            check.checks += 1;
            eprintln!("initial check: {} differences", check.differences.len());
        }

        // Replay.
        let mut lat: BTreeMap<String, Vec<f64>> = BTreeMap::new();
        let mut per_event = Vec::new();
        let mut edits_done = 0usize;
        let mut rss_samples = vec![json!({"at": "loaded", "rss_mb": mb(rss_loaded.0), "footprint_mb": mb(rss_loaded.1)})];
        let mut fact_load_s = 0.0;
        // Hot keys (with --track): per ref name, Δc_w(name) for every worktree; the R1 presence
        // reduce at key `name` holds the base count plus one value per worktree listed here.
        let mut refc: rustc_hash::FxHashMap<Sym, BTreeMap<W, i32>> = Default::default();
        let bump = |refc: &mut rustc_hash::FxHashMap<Sym, BTreeMap<W, i32>>, wd: &World, w: W, c: Cid, s: i32| {
            for n in wd.r1_refs(c) {
                let e = refc.entry(n).or_default();
                *e.entry(w).or_default() += s;
                if e[&w] == 0 {
                    e.remove(&w);
                }
            }
        };
        if track_on {
            let wd = world.borrow();
            for (w, m) in &members {
                for ((_, c), s) in delta_of(m) {
                    bump(&mut refc, &wd, *w, c, s);
                }
            }
        }
        let mut fanin_lat: Vec<(usize, f64)> = Vec::new();
        for ev in &events {
            let w = ev.worktree;
            // Facts of new contents come from disk before the input is offered (not timed).
            let tl = Instant::now();
            let mut ups: Vec<((W, (Sym, Cid)), flow::R)> = Vec::new();
            {
                let mut wd = world.borrow_mut();
                if ev.group == "new_worktree" || ev.group == "commit_init" {
                    assert!(!members.contains_key(&w), "worktree {w} exists");
                    members.insert(w, base_map.clone());
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
            fact_load_s += tl.elapsed().as_secs_f64();
            // fan-in: the widest presence-reduce key this event touches (worktrees at that key).
            let mut fanin = 0usize;
            if track_on {
                let wd = world.borrow();
                let mut touched: BTreeMap<Sym, i32> = BTreeMap::new();
                for ((_, (_, c)), s) in &ups {
                    for n in wd.r1_refs(*c) {
                        *touched.entry(n).or_default() += s;
                    }
                }
                for ((ww, (_, c)), s) in &ups {
                    bump(&mut refc, &wd, *ww, *c, *s);
                }
                for (n, d) in touched {
                    if d != 0 {
                        fanin = fanin.max(refc.get(&n).map(|m| m.len()).unwrap_or(0));
                    }
                }
            }
            let t0 = Instant::now();
            for (r, s) in ups {
                h.files.update(r, s);
            }
            epoch += 1;
            h.files.advance_to(epoch);
            h.files.flush();
            worker.step_while(|| h.probe.less_than(&epoch));
            let ms_ = t0.elapsed().as_secs_f64() * 1e3;
            lat.entry(ev.group.clone()).or_default().push(ms_);
            if ev.group == "edit" {
                lat.entry(format!("edit:{}", ev.kind)).or_default().push(ms_);
            }
            per_event.push(json!([ev.seq, ev.group, ev.kind, w, ev.changes.len(), ms_, fanin]));
            if track_on && (ev.group == "edit" || ev.group == "new_worktree_edit") {
                fanin_lat.push((fanin, ms_));
            }
            if ev.group == "edit" {
                edits_done += 1;
            }
            if oracle_on {
                check_wt(w, &members, &h, &mut check, ev.seq);
                check.checks += 1;
                if ev.group == "edit" && edits_done % 100 == 0 {
                    for ww in members.keys().copied().collect::<Vec<_>>() {
                        check_wt(ww, &members, &h, &mut check, ev.seq);
                    }
                    eprintln!("edit {edits_done}: all-worktree check, {} differences so far", check.differences.len());
                }
                if !check.differences.is_empty() && check.differences.len() > 50 {
                    eprintln!("stopping: too many differences");
                    break;
                }
            }
            if ev.group == "edit" && edits_done % 100 == 0 {
                let r = rss();
                rss_samples.push(json!({"at": format!("edit {edits_done}"), "rss_mb": mb(r.0), "footprint_mb": mb(r.1)}));
            }
        }
        let rss_end = rss();
        rss_samples.push(json!({"at": "end", "rss_mb": mb(rss_end.0), "footprint_mb": mb(rss_end.1)}));

        // Per-worktree state (with --track): live delta records per relation, bytes = records x
        // in-memory update size.
        let tr = h.tracker.borrow();
        let mut per_wt: BTreeMap<W, (BTreeMap<&str, usize>, usize)> = BTreeMap::new();
        for ((name, w, _), _) in tr.live.iter() {
            let e = per_wt.entry(*w).or_default();
            *e.0.entry(name).or_default() += 1;
            e.1 += tr.size[name];
        }
        let mut state_rows = Vec::new();
        for (w, m) in &members {
            let d = delta_of(m);
            let paths: BTreeSet<Sym> = d.iter().map(|((p, _), _)| *p).collect();
            let (rels, bytes) = per_wt.get(w).cloned().unwrap_or_default();
            state_rows.push(json!({"worktree": w, "delta_paths": paths.len(), "delta_entries": d.len(), "state_bytes": bytes, "records": rels}));
        }
        drop(tr);
        // Arrangement sizes from differential's logging, grouped by operator name.
        let sz = sizes.borrow();
        let mut by_name: BTreeMap<String, (isize, isize)> = BTreeMap::new();
        for (op, n) in &sz.records {
            let name = sz.names.get(op).cloned().unwrap_or_else(|| format!("op{op}"));
            by_name.entry(name).or_default().0 += n;
        }
        for (op, b) in &sz.batcher_bytes {
            let name = sz.names.get(op).cloned().unwrap_or_else(|| format!("op{op}"));
            by_name.entry(name).or_default().1 += b;
        }
        let total_arranged: isize = by_name.values().map(|v| v.0).sum();
        let mut arr: Vec<(String, (isize, isize))> = by_name.into_iter().collect();
        arr.sort_by_key(|x| -x.1 .0);
        let arrangements = json!({"total_records": total_arranged, "by_operator": arr.iter().map(|(n, (r, b))| json!([n, r, b])).collect::<Vec<_>>()});
        // Hot keys: names whose R1 presence key holds deltas from many worktrees, and the latency
        // of single-file edits grouped by the widest key they touch.
        let hot = if track_on {
            let wd = world.borrow();
            let mut keys: Vec<(usize, Sym)> = refc.iter().map(|(n, m)| (m.len(), *n)).filter(|x| x.0 > 0).collect();
            keys.sort_unstable_by(|a, b| b.cmp(a));
            let mut hist: BTreeMap<usize, usize> = BTreeMap::new();
            for (k, _) in &keys {
                *hist.entry(*k).or_default() += 1;
            }
            let mut buckets: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
            for (f, l) in &fanin_lat {
                let b = match f { 0 => "0", 1..=5 => "1-5", 6..=20 => "6-20", 21..=40 => "21-40", _ => ">40" };
                buckets.entry(b).or_default().push(*l);
            }
            json!({
                "top": keys.iter().take(25).map(|(k, n)| json!([wd.store.syms.str(*n), k])).collect::<Vec<_>>(),
                "keys_by_worktree_count": hist,
                "edit_latency_by_fanin": buckets.iter().map(|(b, v)| (b.to_string(), summary(v))).collect::<serde_json::Map<_, _>>(),
            })
        } else {
            json!(null)
        };
        let wd = world.borrow();
        let member_hashes: BTreeSet<Cid> = members.values().flat_map(|m| m.values().copied()).chain(base_map.values().copied()).collect();
        let fact_bytes: usize = member_hashes.iter().map(|c| wd.store.contents[*c as usize].bytes()).sum();
        let mut lat_summary = serde_json::Map::new();
        for (k, v) in &lat {
            lat_summary.insert(k.clone(), summary(v));
        }
        let single: Vec<f64> = ["edit", "new_worktree_edit"].iter().flat_map(|k| lat.get(*k).cloned().unwrap_or_default()).collect();
        lat_summary.insert("single_file_all".into(), summary(&single));
        let out = json!({
            "oracle": oracle_on,
            "events": events.len(),
            "load": {"store_seconds": load_store_s, "base_epoch_seconds": base_s, "deltas_epoch_seconds": deltas_s,
                     "rss_after_store_mb": mb(rss_after_store.0), "rss_after_base_mb": mb(rss_after_base.0), "rss_loaded_mb": mb(rss_loaded.0)},
            "base_findings": {"r1": base_counts.0, "r2": base_counts.1, "r3": base_counts.2},
            "initial_findings_per_worktree": sizes0.iter().map(|(w, s)| json!([w, s.0, s.1, s.2])).collect::<Vec<_>>(),
            "latency": lat_summary,
            "fact_load_seconds_total": fact_load_s,
            "rss": rss_samples,
            "exactness": {"checks": check.checks, "worktree_checks": check.worktree_checks, "differences": check.differences.len(),
                          "first_differences": check.differences.iter().take(20).collect::<Vec<_>>(), "oracle_seconds": check.oracle_seconds},
            "state": state_rows,
            "tracked": track_on,
            "arrangements": arrangements,
            "per_worktree_load_ms": per_wt_load_ms,
            "resident_facts": {"contents": member_hashes.len(), "bytes": fact_bytes, "interned_strings": wd.store.syms.len(), "interner_bytes": wd.store.syms.bytes()},
            "divergence_initial": divergence.iter().map(|(w, d)| json!([w, d.0, d.1])).collect::<Vec<_>>(),
            "hot_keys": hot,
        });
        std::fs::write(run_dir.join("results.json"), serde_json::to_string_pretty(&out).unwrap()).unwrap();
        std::fs::write(run_dir.join("events.json"), serde_json::to_string(&per_event).unwrap()).unwrap();
        println!("{}", serde_json::to_string_pretty(&json!({"latency": out["latency"], "rss": out["rss"], "exactness": out["exactness"], "load": out["load"], "base_findings": out["base_findings"], "arranged_records": out["arrangements"]["total_records"]})).unwrap());
    });
}
