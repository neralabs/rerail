//! LEG-083 on the real data-layer repository: the graph and the answers of four versions of the
//! script's search, written by ../realgraph.py. For each version, the set-based plan on the rewritten
//! graph (legacy surfaces blocked) must give the script's answer on every root:
//!   R  the script as it is                         dfs_first_memo, eager = any .resolve()
//!   S  a fresh memo per root (the memo fix)        dfs_first,      eager = any .resolve()
//!   U  the corrected eager test                    dfs_first_memo, eager = lazy terminals only
//!   T  both fixes (the recommended LEG-083)        dfs_first,      eager = lazy terminals only
//! The oracle (ordered_first.rs, legacy surfaces as its supplied visiting set) is checked against R.
//!
//! File lines: `F path`, `N id legacy sinkline symbol`, `K id lazyline line:kind…`,
//! `E src dst ord line`, and per version `R|S|U|T root -` or `… root line eagerid`.

use crate::{Found, Graph, NONE, Run, cpu, oracle, par, prepare, rewrite};
use std::collections::{HashMap, HashSet};

const VERSIONS: [(&str, &str, bool, bool); 4] = [
    ("R", "script", true, false),
    ("S", "fresh memo", false, false),
    ("U", "corrected eager test", true, true),
    ("T", "both fixes", false, true),
];

pub(crate) fn real(path: &str, threads: usize) -> bool {
    let t = cpu();
    let text = std::fs::read_to_string(path).expect("read graph");
    let (mut n, mut files) = (0usize, 0usize);
    let (mut edges, mut sinks, mut lazy) = (Vec::new(), Vec::new(), Vec::new());
    let mut legacy = HashSet::new();
    let mut want: HashMap<&str, (Vec<u32>, Vec<Found>)> = HashMap::new();
    let mut kinds: HashMap<String, usize> = HashMap::new();
    for line in text.lines() {
        let mut w = line.split(' ');
        let tag = w.next().unwrap_or("");
        match tag {
            "F" => files += 1,
            "N" => {
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
            "K" => {
                let id: u32 = w.next().unwrap().parse().unwrap();
                let l: i64 = w.next().unwrap().parse().unwrap();
                if l >= 0 {
                    lazy.push((id, l));
                }
                for k in w {
                    *kinds.entry(k.split(':').nth(1).unwrap().to_string()).or_default() += 1;
                }
            }
            "E" => {
                let v: Vec<i64> = w.map(|x| x.parse().unwrap()).collect();
                edges.push((v[0] as u32, v[1] as u32, v[2], v[3]));
            }
            "R" | "S" | "U" | "T" => {
                let r: u32 = w.next().unwrap().parse().unwrap();
                let e = want.entry(VERSIONS.iter().find(|v| v.0 == tag).unwrap().0).or_default();
                e.0.push(r);
                e.1.push(match w.next() {
                    Some("-") => None,
                    Some(l) => Some((w.next().unwrap().parse().unwrap(), l.parse().unwrap())),
                    None => panic!("bad answer line"),
                });
            }
            _ => {}
        }
    }
    let found = |v: &[Found]| v.iter().filter(|f| f.is_some()).count();
    let mut kinds: Vec<_> = kinds.into_iter().collect();
    kinds.sort();
    println!("{path}: {files} files, {n} functions, {} resolved calls, {} legacy surfaces; \
              .resolve() calls by kind {kinds:?}  (parse {:.3}s)",
             edges.len(), legacy.len(), cpu() - t);
    let (roots, script) = &want["R"];

    // The oracle with the script's state: legacy surfaces are the supplied visiting set.
    let g = Graph::new(n, edges.clone(), &sinks);
    let t = cpu();
    let (or, steps) = oracle(&g, roots, true, true, &legacy, &HashMap::new());
    let mut ok = or == *script;
    println!("  oracle, shared memo           {:7.3}s, {steps} steps, {}", cpu() - t, verdict(&or, script));

    for (sink_set, label) in [(&sinks, "any .resolve()"), (&lazy, "lazy terminals")] {
        let t = cpu();
        let (e2, s2) = rewrite(n, &edges, sink_set, true, &legacy, &HashMap::new());
        let g2 = Graph::new(n, e2, &s2);
        let t_load = cpu() - t;
        let mut times = Vec::new();
        let p = prepare(&g2, &mut times);
        let pass: f64 = times.iter().map(|t| t.1).sum();
        let knot = p.knot.iter().filter(|&&k| k).count();
        let meets = roots.iter().filter(|&&r| p.ent[r as usize] != NONE && p.live[r as usize]).count();
        println!("  eager = {label}: {} targets; rewrite + CSR {t_load:.3}s; set passes {pass:.3}s; live {}, knot {knot}, roots meeting it {meets}",
                 s2.len(), p.live.iter().filter(|&&l| l).count());
        for &(tag, name, share, fixed) in &VERSIONS {
            if fixed != (label == "lazy terminals") {
                continue;
            }
            let (r2, w2) = &want[tag];
            assert!(r2 == roots, "versions list different roots");
            let t = cpu();
            let mut run = Run { g: &g2, p: &p, steps: 0, searches: 0 };
            let got = run.run(roots, true, share);
            let dt = cpu() - t;
            let v = verdict(&got, w2);
            ok &= v.starts_with("agrees");
            println!("    {tag} {name:22} {:5} findings  set-based {dt:6.3}s, {:6} search steps, {v}",
                     found(w2), run.steps);
        }
        let mut tp = Vec::new();
        let pp = par::prepare_par(&g2, threads, &mut tp);
        ok &= par::same(&p, &pp);
        print!("    {threads} threads: set passes {:.3}s CPU, arrays {}", tp.iter().map(|t| t.1).sum::<f64>(),
               if par::same(&p, &pp) { "equal" } else { "DIFFER" });
        if let Some(dev) = crate::gpu::init() {
            let o = crate::gpu::prepare(&g2);
            let d = crate::gpu::differs(&o, &p);
            ok &= d.is_none();
            print!("; GPU ({dev}) steps 1-3 {:.4}s, {}", o.gpu_seconds, d.map_or("arrays equal".into(), |x| format!("{x} DIFFERS")));
        }
        println!();
    }
    ok
}

fn verdict(got: &[Found], want: &[Found]) -> String {
    let bad = got.iter().zip(want).filter(|(a, b)| a != b).count();
    if bad == 0 { "agrees with the script on every root".into() } else { format!("DISAGREES with the script on {bad} roots") }
}
