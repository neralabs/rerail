//! LEG-083 on the real data-layer repository: the graph and the script's own answers, written by
//! ../realgraph.py. Three answers per root must agree:
//!   - the script's `_transitive_eager_resolve` (R lines, computed in Python by a copy of it);
//!   - the oracle, node mode, legacy surfaces as the supplied visiting set, one shared memo;
//!   - the set-based plan on the rewritten graph (legacy surfaces blocked), `dfs_first_memo`.
//! It also reports what the fresh-memo `dfs_first` (LEG_083_SOUND) adds.
//!
//! File lines: `F path`, `N id legacy sinkline symbol`, `E src dst ord line`,
//! `R root -` or `R root line eagerid`.

use crate::{Found, Graph, NONE, Run, cpu, oracle, par, prepare, rewrite};
use std::collections::{HashMap, HashSet};

pub(crate) fn real(path: &str, threads: usize) -> bool {
    let t = cpu();
    let text = std::fs::read_to_string(path).expect("read graph");
    let (mut n, mut files) = (0usize, 0usize);
    let (mut edges, mut sinks, mut roots, mut want) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut legacy = HashSet::new();
    for line in text.lines() {
        let mut w = line.split(' ');
        match w.next() {
            Some("F") => files += 1,
            Some("N") => {
                let id: u32 = w.next().unwrap().parse().unwrap();
                if w.next() == Some("1") {
                    legacy.insert(id);
                }
                let s: i64 = w.next().unwrap().parse().unwrap();
                if s >= 0 {
                    sinks.push((id, s));
                }
                n = n.max(id as usize + 1);
            }
            Some("E") => {
                let v: Vec<i64> = w.map(|x| x.parse().unwrap()).collect();
                edges.push((v[0] as u32, v[1] as u32, v[2], v[3]));
            }
            Some("R") => {
                let r: u32 = w.next().unwrap().parse().unwrap();
                roots.push(r);
                want.push(match w.next() {
                    Some("-") => None,
                    Some(l) => Some((w.next().unwrap().parse().unwrap(), l.parse().unwrap())),
                    None => panic!("bad R line"),
                });
            }
            _ => {}
        }
    }
    let found = |v: &[Found]| v.iter().filter(|f| f.is_some()).count();
    println!("{path}: {files} files, {n} functions, {} resolved calls, {} eager, {} legacy surfaces, {} roots; \
              script: {} findings  (parse {:.3}s)",
             edges.len(), sinks.len(), legacy.len(), roots.len(), found(&want), cpu() - t);

    // The oracle with the script's state: legacy surfaces are the supplied visiting set.
    let g = Graph::new(n, edges.clone(), &sinks);
    let t = cpu();
    let (or, steps) = oracle(&g, &roots, true, true, &legacy, &HashMap::new());
    println!("  oracle, shared memo          {:7.3}s, {steps} steps, {}", cpu() - t, verdict(&or, &want));

    // The set-based plan on the rewrite: blocked = legacy, no seeds.
    let t = cpu();
    let (e2, s2) = rewrite(n, &edges, &sinks, true, &legacy, &HashMap::new());
    let g2 = Graph::new(n, e2, &s2);
    let t_load = cpu() - t;
    let mut times = Vec::new();
    let p = prepare(&g2, &mut times);
    let pass: f64 = times.iter().map(|t| t.1).sum();
    let knot = p.knot.iter().filter(|&&k| k).count();
    let meets = roots.iter().filter(|&&r| p.ent[r as usize] != NONE && p.live[r as usize]).count();
    println!("  rewrite + CSR {t_load:.3}s; set passes {pass:.3}s ({}); live {}, knot {knot}, roots meeting it {meets}",
             times.iter().map(|(n, s)| format!("{n} {s:.3}")).collect::<Vec<_>>().join(", "),
             p.live.iter().filter(|&&l| l).count());
    let mut ok = true;
    for share in [true, false] {
        let t = cpu();
        let mut r = Run { g: &g2, p: &p, steps: 0, searches: 0 };
        let got = r.run(&roots, true, share);
        let dt = cpu() - t;
        if share {
            let v = verdict(&got, &want);
            ok &= v.starts_with("agrees");
            println!("  set-based, shared memo       {dt:7.3}s, {} search steps, {v}", r.steps);
        } else {
            let (fresh, _) = oracle(&g, &roots, true, false, &legacy, &HashMap::new());
            let same = fresh == got;
            ok &= same;
            let added = got.iter().zip(&want).filter(|(a, b)| a.is_some() && b.is_none()).count();
            let moved = got.iter().zip(&want).filter(|(a, b)| a.is_some() && b.is_some() && a != b).count();
            println!("  set-based, fresh memo        {dt:7.3}s, {} search steps, {} oracle; \
                      {} findings: {added} more than the script's, {moved} at another call",
                     r.steps, if same { "agrees with the" } else { "DISAGREES with the" }, found(&got));
        }
    }
    let (pp, tp) = timed_par(&g2, threads);
    ok &= par::same(&p, &pp);
    println!("  {threads} threads: set passes {tp:.3}s CPU, arrays {}", if par::same(&p, &pp) { "equal" } else { "DIFFER" });
    if let Some(dev) = crate::gpu::init() {
        let o = crate::gpu::prepare(&g2);
        let d = crate::gpu::differs(&o, &p);
        ok &= d.is_none();
        println!("  GPU ({dev}): steps 1-3 {:.4}s GPU, {} waves, {} command buffers, {}",
                 o.gpu_seconds, o.waves, o.command_buffers, d.map_or("arrays equal".into(), |x| format!("{x} DIFFERS")));
    }
    ok &= or == want;
    ok
}

fn timed_par(g: &Graph, threads: usize) -> (crate::Prep, f64) {
    let mut times = Vec::new();
    let p = par::prepare_par(g, threads, &mut times);
    (p, times.iter().map(|t| t.1).sum())
}

fn verdict(got: &[Found], want: &[Found]) -> String {
    let bad = got.iter().zip(want).filter(|(a, b)| a != b).count();
    if bad == 0 { "agrees with the script on every root".into() } else { format!("DISAGREES with the script on {bad} roots") }
}
