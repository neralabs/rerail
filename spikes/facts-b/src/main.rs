//! Spike B (facts engine, Phase 1): base + per-worktree delta dataflow for cross-file rules.
//! Throwaway code; the numbers and the schema are what matter. See docs/facts/reports/spike-b.md.
mod base;
mod derive;
mod edits;
mod extract;
mod facts;
mod flow;
mod gen;
mod membership;
mod mutant;
mod oracle;
mod replay;
mod rules;
mod store;
mod y2;
mod ygraph;

use std::path::PathBuf;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("");
    match cmd {
        "extract" => {
            let corpus = PathBuf::from(arg(&args, "--corpus").unwrap_or_else(|| store::home().join("tempo-qe/workspaces47").display().to_string()));
            let run_dir = PathBuf::from(arg(&args, "--run-dir").expect("--run-dir"));
            let threads = arg(&args, "--threads").map(|t| t.parse().unwrap()).unwrap_or(4);
            extract::run(&corpus, &run_dir, threads);
        }
        "replay" => replay::run(
            &PathBuf::from(arg(&args, "--run-dir").expect("--run-dir")),
            replay::Opts {
                oracle: !args.iter().any(|a| a == "--no-oracle"),
                events: arg(&args, "--events").map(PathBuf::from),
                limit: arg(&args, "--limit").map(|x| x.parse().unwrap()),
                track: args.iter().any(|a| a == "--track"),
                clusters: args.iter().any(|a| a == "--clusters"),
                only_r1: args.iter().any(|a| a == "--only-r1"),
                naive: args.iter().any(|a| a == "--naive"),
                plurality: args.iter().any(|a| a == "--plurality"),
            },
        ),
        "greplay" => gen::drive::run(
            &PathBuf::from(arg(&args, "--run-dir").expect("--run-dir")),
            gen::drive::Opts {
                events: arg(&args, "--events").map(PathBuf::from),
                limit: arg(&args, "--limit").map(|x| x.parse().unwrap()),
                oracle: !args.iter().any(|a| a == "--no-oracle"),
                fail_fast: args.iter().any(|a| a == "--fail-fast"),
                clusters: !args.iter().any(|a| a == "--flat"),
                track: args.iter().any(|a| a == "--track"),
                plans: arg(&args, "--plans").map(|s| s.split(',').map(str::to_owned).collect()).unwrap_or_default(),
                real: arg(&args, "--real").map(PathBuf::from),
                parity: arg(&args, "--parity").map(PathBuf::from),
                reference_scratch: match arg(&args, "--scratch").as_deref() {
                    None | Some("engine") => false,
                    Some("reference") => true,
                    Some(o) => panic!("--scratch {o}: engine | reference"),
                },
                check_every: arg(&args, "--check-every").map_or(1, |x| x.parse().unwrap()),
                sweep: arg(&args, "--sweep").map(|x| x.parse().unwrap()),
            },
        ),
        "hreplay" => gen::hybrid::run(
            &PathBuf::from(arg(&args, "--run-dir").expect("--run-dir")),
            gen::hybrid::Opts {
                events: arg(&args, "--events").map(PathBuf::from),
                limit: arg(&args, "--limit").map(|x| x.parse().unwrap()),
                real: PathBuf::from(arg(&args, "--real").expect("--real")),
                check: match arg(&args, "--check").as_deref() {
                    None | Some("batch") => gen::hybrid::Check::Batch,
                    Some("engine") => gen::hybrid::Check::Engine,
                    Some("none") => gen::hybrid::Check::None,
                    Some(o) => panic!("--check {o}: none | batch | engine"),
                },
                check_every: arg(&args, "--check-every").map_or(1, |x| x.parse().unwrap()),
                sweep: arg(&args, "--sweep").map(|x| x.parse().unwrap()),
                skip: !args.iter().any(|a| a == "--no-skip"),
                worktrees: arg(&args, "--worktrees").map(|x| x.parse().unwrap()),
                workers: arg(&args, "--workers").map_or(1, |x| x.parse().unwrap()),
            },
        ),
        "edits" => edits::run(
            &PathBuf::from(arg(&args, "--out").expect("--out")),
            arg(&args, "--seed").map(|x| x.parse().unwrap()).unwrap_or(0x5EED_B),
            &store::home().join("tempo-qe/workspaces47"),
            arg(&args, "--commits-repo").map(PathBuf::from).as_deref(),
        ),
        "hops" => hops(),
        // `b3sum FILE…`: content hashes as the store keys them (scripts/jobs.py).
        "b3sum" => {
            for p in &args[2..] {
                println!("{}  {p}", blake3::hash(&std::fs::read(p).unwrap()).to_hex());
            }
        }
        "divergence" => divergence(&PathBuf::from(arg(&args, "--run-dir").expect("--run-dir")), args.iter().any(|a| a == "--plurality")),
        "dump" => {
            let p = args.get(2).expect("file");
            let src = std::fs::read(p).unwrap();
            let mut parser = facts::parser();
            let tree = facts::parse(&mut parser, &src);
            let e = facts::extract(&src, &tree);
            let h: [u8; 32] = blake3::hash(&src).into();
            let enc = facts::encode(&h, &e);
            let (h2, d) = facts::decode(&enc);
            assert_eq!(h, h2);
            assert_eq!(d, e, "round trip");
            println!("{e:#?}\nencoded bytes: {}", enc.len());
        }
        // `gnorm --real SPEC [--dump STAGE]`: compile and normalise the spec's repository plans
        // only (no exports read): the refusals, and the IR of the named stages.
        "gnorm" => gen::real::check(&PathBuf::from(arg(&args, "--real").expect("--real")), arg(&args, "--dump").as_deref()),
        // `y2 --dump FILE --out DIR [--scratch-every N] [--edges Y1_FILE] | --fuzz N --seed S`: Y2's local-query rules over a greplay
        // stage dump (`FACTS_Y2_DUMP`), scoped per event and checked against scratch and the dataflow.
        "y2" => match arg(&args, "--fuzz") {
            Some(n) => y2::fuzz(
                &PathBuf::from(arg(&args, "--dump").expect("--dump")),
                &PathBuf::from(arg(&args, "--out").expect("--out")),
                n.parse().unwrap(),
                arg(&args, "--seed").map_or(1, |x| x.parse().unwrap()),
            ),
            None => y2::run(
                &PathBuf::from(arg(&args, "--dump").expect("--dump")),
                &PathBuf::from(arg(&args, "--out").expect("--out")),
                arg(&args, "--scratch-every").map_or(1, |x| x.parse().unwrap()),
                arg(&args, "--edges").map(PathBuf::from).as_deref(),
            ),
        },
        // `ygraph parity|replay …`: Y1's maintained reference graph (src/ygraph).
        "ygraph" => ygraph::main(args.clone()),
        _ => {
            eprintln!("usage: facts-b extract --run-dir DIR [--corpus DIR] [--threads N] | dump FILE");
            std::process::exit(2);
        }
    }
}

fn content_rows(c: &store::Content) -> usize {
    c.defs.len() + c.refs.len() + c.imports.len() + c.calls.len() + c.decorators.len() + c.dunder_all_names.len() + c.seeds.len() + c.dunder_all.is_some() as usize
}

/// Step 2: base, deltas, divergence per worktree (files and fact rows).
fn divergence(run_dir: &std::path::Path, plurality: bool) {
    std::fs::create_dir_all(run_dir).unwrap();
    let ms = base::load_memberships();
    let b = if plurality { base::plurality_base(&ms) } else { base::majority_base(&ms) };
    let mut st = store::Store::default();
    let sd = store::store_dir();
    let base_rows: usize = b.iter().map(|(_, h)| { let c = st.load(&sd, h); content_rows(&st.contents[c as usize]) }).sum();
    let mut rows = Vec::new();
    for (w, m) in &ms {
        let d = base::delta(&b, m);
        let paths: std::collections::BTreeSet<&str> = d.iter().map(|(p, _, _)| p.as_str()).collect();
        let added = d.iter().filter(|x| x.2 > 0).count();
        let removed = d.iter().filter(|x| x.2 < 0).count();
        let fact_rows: usize = d.iter().map(|(_, h, _)| { let c = st.load(&sd, h); content_rows(&st.contents[c as usize]) }).sum();
        // Net fact divergence: per path, the multiset symmetric difference of exported rows
        // between the base content and the worktree's (rows equal on both sides cancel).
        let mut by_path: std::collections::BTreeMap<&str, (Option<u32>, Option<u32>)> = Default::default();
        for (p, h, s) in &d {
            let c = st.load(&sd, h);
            let e = by_path.entry(p.as_str()).or_default();
            if *s > 0 { e.1 = Some(c) } else { e.0 = Some(c) }
        }
        let net: usize = by_path.values().map(|(a, b)| {
            let ra = a.map(|c| fact_tuples(&st.contents[c as usize])).unwrap_or_default();
            let rb = b.map(|c| fact_tuples(&st.contents[c as usize])).unwrap_or_default();
            sym_diff(&ra, &rb)
        }).sum();
        rows.push(serde_json::json!({"worktree": w, "files": m.len(), "delta_paths": paths.len(), "delta_plus": added, "delta_minus": removed, "delta_fact_rows": fact_rows, "net_delta_fact_rows": net}));
    }
    let out = serde_json::json!({"base_files": b.len(), "base_fact_rows": base_rows, "worktrees": rows});
    std::fs::write(run_dir.join("results.json"), serde_json::to_string_pretty(&out).unwrap()).unwrap();
    let name = if plurality { "base-plurality.tsv" } else { "base.tsv" };
    store::write_membership(&store::spike_dir().join("membership").join(name), &b);
    for r in out["worktrees"].as_array().unwrap() {
        println!("{} files={} delta_paths={} +{} -{} fact_rows={} net_fact_rows={}", r["worktree"], r["files"], r["delta_paths"], r["delta_plus"], r["delta_minus"], r["delta_fact_rows"], r["net_delta_fact_rows"]);
    }
    println!("base files {} base fact rows {}", b.len(), base_rows);
}

/// A content's exported rows as sorted (relation, fields) tuples.
fn fact_tuples(c: &store::Content) -> Vec<(u8, [u32; 4])> {
    let mut v: Vec<(u8, [u32; 4])> = Vec::new();
    v.extend(c.defs.iter().map(|d| (0, [d.0, d.1, d.2, d.3])));
    v.extend(c.refs.iter().map(|r| (1, [*r, 0, 0, 0])));
    v.extend(c.imports.iter().map(|i| (2, [i.module, i.name, i.local, (i.top as u32) | (i.module_scope as u32) << 1])));
    v.extend(c.calls.iter().map(|x| (3, [x.0, x.1, 0, 0])));
    v.extend(c.decorators.iter().map(|x| (4, [x.0, x.1, 0, 0])));
    v.extend(c.dunder_all.iter().map(|x| (5, [*x as u32, 0, 0, 0])));
    v.extend(c.dunder_all_names.iter().map(|x| (6, [*x, 0, 0, 0])));
    v.extend(c.seeds.iter().map(|x| (7, [x.0, x.1, 0, 0])));
    v.sort_unstable();
    v
}

fn sym_diff(a: &[(u8, [u32; 4])], b: &[(u8, [u32; 4])]) -> usize {
    let (mut i, mut j, mut n) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Equal => { i += 1; j += 1; }
            std::cmp::Ordering::Less => { n += 1; i += 1; }
            std::cmp::Ordering::Greater => { n += 1; j += 1; }
        }
    }
    n + (a.len() - i) + (b.len() - j)
}

/// R2 hop census over the base and the 40 worktrees' initial memberships.
fn hops() {
    let ms = base::load_memberships();
    let b = store::read_membership(&store::spike_dir().join("membership/base.tsv"));
    let mut st = store::Store::default();
    let sd = store::store_dir();
    for (_, h) in b.iter().chain(ms.iter().flat_map(|(_, m)| m.iter())) {
        st.load(&sd, h);
    }
    let mut world = derive::World::new(st);
    let mut total: std::collections::BTreeMap<String, usize> = Default::default();
    let mut open_total = 0;
    for (name, m) in std::iter::once(("base".to_string(), &b)).chain(ms.iter().map(|(n, m)| (n.clone(), m))) {
        let files: Vec<(u32, u32)> = m.iter().map(|(p, h)| (world.path(p), world.store.by_hash[h])).collect();
        let (hist, open) = oracle::r2_hop_census(&mut world, &files);
        for (k, v) in &hist {
            let key = if *k == usize::MAX { "unresolved".to_string() } else { format!("{k}") };
            *total.entry(key).or_default() += v;
        }
        open_total += open;
        if name == "base" || name == "01" || name == "40" {
            println!("{name}: {hist:?} open {open}");
        }
    }
    println!("all 41 memberships: hops -> roots {total:?}, cut off after 64: {open_total}");
}
