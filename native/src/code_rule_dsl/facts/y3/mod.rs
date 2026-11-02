//! Y3: Y1's maintained graph and Y2's local rules wired into one cross-file path, with the
//! symbol table maintained in Y1's style (`sym`) instead of the dataflow, and the three small
//! plans through P3a's M2 recompute with the D1 skip (`m2`). `wire` is the per-edit path,
//! `world` the model behind the daemon's `CrossFile` interface. See
//! docs/facts/reports/explore-y3-wire.md.
//!
//! - `facts-b y3 symcheck …`: the maintained symbol table against the dataflow's stages (a Y2
//!   stage dump: the oracle tree from scratch, the replay prefix per event).
//! - `facts-b y3 replay …`: the wired path on S3's replay, measured and checked from scratch.
//! - `facts-b y3 spec FILE`: the M2 spec of the three small plans.
pub mod input;
pub mod m2;
pub mod replay;
pub mod sym;
pub mod wire;
pub mod world;

use crate::y2::facts::{show_row, string, sym, Change, Counts, Row, Tab, Val};
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;
use sym::{PathChange, SymShared, SymState};

pub fn bench() -> PathBuf {
    crate::ygraph::bench()
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

pub fn main(args: Vec<String>) {
    match args.get(2).map(String::as_str) {
        Some("symcheck") => crate::ygraph::run_big(move || symcheck(&args)),
        Some("replay") => crate::ygraph::run_big(move || replay::run(&args)),
        Some("spec") => replay::write_spec(Path::new(args.get(3).expect("y3 spec FILE"))),
        _ => {
            eprintln!("usage: facts-b y3 symcheck|replay|spec [...]");
            std::process::exit(2);
        }
    }
}

/// One replay event (S3's `events.jsonl`).
pub struct Event {
    pub seq: u64,
    pub worktree: u32,
    pub group: String,
    pub kind: String,
    pub changes: Vec<(String, Option<String>, Option<String>)>,
}

pub fn read_events(p: &Path) -> Vec<Event> {
    std::fs::read_to_string(p)
        .unwrap_or_else(|e| panic!("{}: {e}", p.display()))
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

/// A dump row as Y3 emits it: `Dyn` rows keep only (path, target) (Y3 reads distinct targets
/// per content); `Refs` and the rule outputs are not the symbol table's.
fn sym_row(t: Tab, r: &Row) -> Option<(Tab, Row)> {
    match t {
        Tab::Refs => None,
        Tab::Dyn => r[3].os().map(|fq| (t, Box::new([r[0], Val::Int(0), Val::Int(0), Val::S(fq)]) as Row)),
        _ => Some((t, r.clone())),
    }
}

fn dump_rows(d: &crate::y2::Dump, rows: &[(usize, i32, Row)]) -> Vec<(Tab, Row, i32)> {
    rows.iter().filter_map(|(k, c, r)| d.tabs[*k].and_then(|t| sym_row(t, r)).map(|(t, r)| (t, r, *c))).collect()
}

/// Rows only on one side, by table: (table, left only, right only, first examples).
fn compare(left: &FxHashSet<(Tab, Row)>, right: &FxHashSet<(Tab, Row)>) -> Vec<serde_json::Value> {
    let mut by: BTreeMap<String, (usize, usize, Vec<String>, Vec<String>)> = BTreeMap::new();
    for x in left.difference(right) {
        let e = by.entry(format!("{:?}", x.0)).or_default();
        e.0 += 1;
        if e.2.len() < 4 {
            e.2.push(show_row(&x.1));
        }
    }
    for x in right.difference(left) {
        let e = by.entry(format!("{:?}", x.0)).or_default();
        e.1 += 1;
        if e.3.len() < 4 {
            e.3.push(show_row(&x.1));
        }
    }
    by.into_iter().map(|(t, (a, b, ea, eb))| json!({"table": t, "y3_only": a, "dataflow_only": b, "y3_examples": ea, "dataflow_examples": eb})).collect()
}

fn present(c: &Counts) -> FxHashSet<(Tab, Row)> {
    c.present().map(|(t, r)| (t, r.clone())).collect()
}

fn read_members(p: &Path) -> Vec<(u32, u32)> {
    crate::ygraph::read_members(p).into_iter().map(|(p, h)| (sym(&p), sym(&h))).collect()
}

fn scratch(sh: &mut SymShared, members: &[(u32, u32)]) -> (SymState, sym::Work) {
    let mut st = SymState::default();
    let ch: Vec<PathChange> = members.iter().map(|&(p, c)| PathChange { path: p, old: None, new: Some(c) }).collect();
    let (_, w) = st.apply(sh, &ch);
    (st, w)
}

/// `y3 symcheck --dump D --out O (--oracle | [--limit N])`
fn symcheck(args: &[String]) {
    let out = PathBuf::from(arg(args, "--out").expect("--out DIR"));
    std::fs::create_dir_all(&out).unwrap();
    let oracle = args.iter().any(|a| a == "--oracle");
    let b = bench();
    let dump_p = PathBuf::from(arg(args, "--dump").expect("--dump FILE"));
    let exports = PathBuf::from(arg(args, "--exports").unwrap_or_else(|| {
        b.join(format!("explore/x6-rules/exports/{}-symbols-merged.jsonl", if oracle { "oracle" } else { "replay" })).display().to_string()
    }));
    let t = Instant::now();
    let d = crate::y2::read_dump(&dump_p);
    eprintln!("symcheck: dump read in {:.1}s", t.elapsed().as_secs_f64());
    let mut sh = SymShared::default();
    let t = Instant::now();
    let oracle_members = oracle.then(|| read_members(&b.join("phase-2/s3-crossfile/exports/oracle-b6-jobs.tsv")));
    let want: Option<FxHashSet<(String, String)>> = oracle_members.as_ref().map(|m| m.iter().map(|(p, h)| (string(*p), string(*h))).collect());
    let n = input::load(&exports, &mut sh.blocks, want.as_ref().map(|w| Box::new(move |p: &str, h: &str| w.contains(&(p.to_owned(), h.to_owned()))) as Box<dyn Fn(&str, &str) -> bool + Sync>).as_deref());
    let load_s = t.elapsed().as_secs_f64();
    eprintln!("symcheck: {n} blocks in {load_s:.1}s ({:.0} MB)", sh.blocks.bytes() as f64 / 1e6);
    let mut report = json!({"dump": dump_p.display().to_string(), "exports": exports.display().to_string(), "blocks": n, "load_s": load_s, "blocks_bytes": sh.blocks.bytes()});
    if let Some(m) = oracle_members {
        let t = Instant::now();
        let (st, w) = scratch(&mut sh, &m);
        let build_s = t.elapsed().as_secs_f64();
        let mine: FxHashSet<(Tab, Row)> = st.all_rows(&sh).into_iter().collect();
        let mut c = Counts::default();
        c.apply(&dump_rows(&d, &d.reference));
        let want = present(&c);
        let diffs = compare(&mine, &want);
        let mut per: BTreeMap<String, usize> = BTreeMap::new();
        for (t, _) in &mine {
            *per.entry(format!("{t:?}")).or_default() += 1;
        }
        eprintln!("symcheck oracle: {} rows y3, {} dataflow, {} tables differ; build {build_s:.2}s", mine.len(), want.len(), diffs.len());
        report["oracle"] = json!({"members": m.len(), "build_s": build_s, "work": w, "rows": per, "differences": diffs, "state": st.bytes(), "queries": sh.qt.len()});
    } else {
        let limit: Option<usize> = arg(args, "--limit").map(|x| x.parse().unwrap());
        let events = read_events(&PathBuf::from(arg(args, "--events").unwrap_or_else(|| b.join("spike-b/edits/seed-5eedb/events.jsonl").display().to_string())));
        let events: Vec<&Event> = events.iter().take(limit.unwrap_or(usize::MAX)).collect();
        let by_seq: FxHashMap<u64, &Event> = events.iter().map(|e| (e.seq, *e)).collect();
        let mdir = b.join("spike-b/membership");
        let mut worktrees: Vec<u32> = d.reference_w.into_iter().chain(d.init.keys().copied()).collect();
        worktrees.sort_unstable();
        worktrees.dedup();
        let (mut checks, mut differences) = (0usize, Vec::new());
        let mut lat: Vec<f64> = Vec::new();
        let mut work = Vec::new();
        for &wt in &worktrees {
            let mut c = Counts::default();
            c.apply(&dump_rows(&d, &d.reference));
            if Some(wt) != d.reference_w {
                c.apply(&dump_rows(&d, d.init.get(&wt).map_or(&[][..], Vec::as_slice)));
            }
            let created = d.events.iter().find(|e| e.w == wt && e.creates);
            let members = if created.is_some() { read_members(&mdir.join("base.tsv")) } else { read_members(&mdir.join(format!("{wt:02}.tsv"))) };
            let (mut st, _) = scratch(&mut sh, &members);
            if let Some(ce) = created {
                let ev = by_seq[&(ce.seq as u64)];
                let ch = path_changes(ev);
                st.apply(&mut sh, &ch);
            }
            let mine: FxHashSet<(Tab, Row)> = st.all_rows(&sh).into_iter().collect();
            let diffs = compare(&mine, &present(&c));
            checks += 1;
            if !diffs.is_empty() {
                differences.push(json!({"worktree": wt, "at": "load", "tables": diffs}));
            }
            let mut n = 0;
            for e in d.events.iter().filter(|e| e.w == wt && !e.creates) {
                let ev = by_seq[&(e.seq as u64)];
                let delta = dump_rows(&d, &e.rows);
                let want_ch: FxHashSet<(Tab, Row, bool)> = c.classify(&delta).into_iter().map(|x| (x.tab, x.row, x.added)).collect();
                c.apply(&delta);
                let (got, w) = st.apply(&mut sh, &path_changes(ev));
                lat.push(w.ms);
                work.push(json!([e.seq, wt, ev.group, w]));
                let got: FxHashSet<(Tab, Row, bool)> = got.into_iter().map(|x: Change| (x.tab, x.row, x.added)).collect();
                checks += 1;
                n += 1;
                if got != want_ch {
                    let only_g: Vec<String> = got.difference(&want_ch).take(6).map(|x| format!("{:?} {} {}", x.0, if x.2 { "+" } else { "-" }, show_row(&x.1))).collect();
                    let only_w: Vec<String> = want_ch.difference(&got).take(6).map(|x| format!("{:?} {} {}", x.0, if x.2 { "+" } else { "-" }, show_row(&x.1))).collect();
                    differences.push(json!({"worktree": wt, "event": e.seq, "y3_only": only_g, "dataflow_only": only_w}));
                }
            }
            // and the whole state at the end of the worktree's events
            let mine: FxHashSet<(Tab, Row)> = st.all_rows(&sh).into_iter().collect();
            let diffs = compare(&mine, &present(&c));
            checks += 1;
            if !diffs.is_empty() {
                differences.push(json!({"worktree": wt, "at": "end", "tables": diffs}));
            }
            eprintln!("symcheck: worktree {wt}: {n} events, {} differences so far", differences.len());
            if differences.len() > 40 {
                break;
            }
        }
        lat.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p = |q: f64| lat.get(((lat.len().max(1) - 1) as f64 * q).round() as usize).copied().unwrap_or(0.0);
        report["replay"] = json!({"worktrees": worktrees.len(), "checks": checks, "differences": differences.len(), "first_differences": differences.iter().take(40).collect::<Vec<_>>(),
            "sym_ms": {"n": lat.len(), "p50": p(0.5), "p95": p(0.95), "p99": p(0.99), "max": p(1.0)}});
        std::fs::write(out.join("work.json"), serde_json::to_string(&work).unwrap()).unwrap();
    }
    std::fs::write(out.join("symcheck.json"), serde_json::to_string_pretty(&report).unwrap()).unwrap();
    let mut short = report.clone();
    if let Some(o) = short.get_mut("replay") {
        o["first_differences"] = json!(o["first_differences"].as_array().map(|a| a.iter().take(5).cloned().collect::<Vec<_>>()));
    }
    println!("{}", serde_json::to_string_pretty(&short).unwrap());
}

pub fn path_changes(ev: &Event) -> Vec<PathChange> {
    ev.changes.iter().map(|(p, o, n)| PathChange { path: sym(p), old: o.as_deref().map(sym), new: n.as_deref().map(sym) }).collect()
}
