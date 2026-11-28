//! `join-lab`: the launches captured under a directory (`RERAIL_JOIN_CAPTURE`'s), apart from the engine.
//!
//! - `join-lab list <capture> [--shapes shapes.yaml]`: each launch, its shapes by work, its regions and bytes; with the
//!   catalogue, each shape by its rows, its class or the part it lacks, and its launches captured.
//! - `join-lab run <capture> [--engine class,program,metal] [--only <text>]`: each launch replayed on each engine, held
//!   to what the engine's launch left; an engine that leaves otherwise is an error naming its first differing region. A
//!   launch whose shapes have no class is listed as such on `class`, not failed.
//! - `join-lab profile <capture> [--engine class,program,metal] [--repeat N] [--core N] [--sample] [--only <text>]
//!   [-o results.tsv]`: each launch run warm N times (loaded afresh each time, the load not timed): its best and median
//!   wall time, the thread's CPU time, instructions and cycles, the program's items by kind, written as a results
//!   table; with `--sample`, each engine's stacks folded beside it (`results.<engine>.folded`).
//! - `join-lab diverge <capture> --engine a,b [--only <text>]`: where two engines' replays of each launch first part:
//!   the first dispatch after which their regions differ, the phase it ran and its items' kinds.
//! - `join-lab script <capture> --engine e [--upto K] [--only <text>]`: each launch's size script as the engine's
//!   load binds it (after its first K dispatches ran), each address placed in the region that holds it (or none).
//! - `join-lab page <results.tsv> [--shapes shapes.yaml] -o page.html`: the page of a profile's results.

use rerail_join_lab::{Engine, Wave};
use std::path::Path;

fn usage() -> ! {
    eprintln!("usage: join-lab list <capture> [--shapes shapes.yaml]\n       join-lab run <capture> [--engine class,program,metal] [--only <text>]\n       join-lab profile <capture> [--engine class,program,metal] [--repeat N] [--core N] [--sample] [--only <text>] [-o results.tsv]\n       join-lab diverge <capture> --engine a,b [--only <text>]\n       join-lab script <capture> --engine e [--upto K] [--only <text>]\n       join-lab page <results.tsv> [--shapes shapes.yaml] -o page.html");
    std::process::exit(2);
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn waves(capture: &str, only: Option<&str>) -> Vec<Wave> {
    let all = Wave::all(Path::new(capture)).unwrap_or_else(|e| {
        eprintln!("join-lab: {capture}: {e}");
        std::process::exit(1);
    });
    all.into_iter().filter(|w| only.is_none_or(|o| w.name() == o || w.labels.iter().any(|l| l.shape.contains(o) || l.rule.contains(o)))).collect()
}

/// The catalogue `--shapes` names (`shapes.yaml`, the profile's).
fn catalogue(args: &[String]) -> Option<std::collections::BTreeMap<String, rerail_join_lab::catalogue::Entry>> {
    let path = flag(args, "--shapes")?;
    match std::fs::read_to_string(path) {
        Ok(t) => Some(rerail_join_lab::catalogue::parse(&t)),
        Err(e) => {
            eprintln!("join-lab: {path}: {e}");
            std::process::exit(1);
        }
    }
}

fn engines(args: &[String]) -> Vec<Engine> {
    flag(args, "--engine").unwrap_or("program").split(',').map(|e| Engine::named(e).unwrap_or_else(|why| {
        eprintln!("join-lab: {why}");
        std::process::exit(2);
    })).collect()
}

fn main() {
    // The binary's run, on a machine of its own (decision 0442): an entry point holds one, and the engine's parts the
    // engines call (the device's pipelines) read their values from it.
    let _run = rerail_context::within(rerail_context::Shared::on(&rerail_context::Machine::new()));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first().map(String::as_str) else { usage() };
    let Some(target) = args.get(1) else { usage() };
    let only = flag(&args, "--only");
    match cmd {
        "list" => {
            println!("{:<16} {:>8} {:>10} {:>12}  shapes (work)", "launch", "regions", "bytes", "unresolved");
            for w in waves(target, only) {
                let shapes: Vec<String> = w.labels.iter().map(|l| format!("{} ({})", l.shape, l.work)).collect();
                println!("{:<16} {:>8} {:>10} {:>12}  {}", w.name(), w.regions.len(), w.bytes(), w.unresolved + w.unbound, shapes.join(" "));
            }
            if let Some(c) = catalogue(&args) {
                // Each shape by its rows into its stages, its class or the part it lacks, its launches captured.
                let all = waves(target, None);
                let mut shapes: Vec<&rerail_join_lab::catalogue::Entry> = c.values().collect();
                shapes.sort_by_key(|e| std::cmp::Reverse(e.rows.iter().map(|r| r[0]).sum::<u64>()));
                println!("\n{:<20} {:>12} {:>6}  {:<26} {:>8}  parts", "shape", "rows in", "runs", "class (or missing part)", "captured");
                for e in shapes {
                    let captured = all.iter().filter(|w| w.labels.iter().any(|l| l.shape == e.name)).count();
                    let class = match e.class() {
                        Ok(c) => c.to_owned(),
                        Err(part) => format!("missing {part}"),
                    };
                    println!("{:<20} {:>12} {:>6}  {:<26} {:>8}  {}", e.name, e.rows.iter().map(|r| r[0]).sum::<u64>(), e.runs, class, captured, e.parts().join(" "));
                }
            }
        }
        "run" => {
            let mut failed = 0;
            for w in waves(target, only) {
                for e in engines(&args) {
                    // Its name first: a replay that faults names its launch.
                    eprintln!("join-lab: {} on {}", w.name(), e.name());
                    if e == Engine::Class
                        && let Err(why) = w.classes()
                    {
                        println!("{:<16} {:<8} none   {why}", w.name(), e.name());
                        continue;
                    }
                    match rerail_join_lab::replay_in(&w, e, args.iter().any(|a| a == "--reverse")) {
                        Ok(r) if r.same() => println!("{:<16} {:<8} same   {:>4} dispatches {:>9} groups {:>9.3} ms", w.name(), e.name(), r.ran.dispatches, r.ran.groups, r.ran.seconds * 1e3),
                        Ok(r) => {
                            failed += 1;
                            println!("{:<16} {:<8} DIFFERS {}", w.name(), e.name(), r.differing());
                        }
                        Err(err) => {
                            failed += 1;
                            println!("{:<16} {:<8} FAILED {err}", w.name(), e.name());
                        }
                    }
                }
            }
            std::process::exit(i32::from(failed > 0));
        }
        "diverge" => {
            // Where two engines' replays first part: the first dispatch after which their regions differ.
            let e = engines(&args);
            let (a, b) = match e.as_slice() {
                [a, b] => (*a, *b),
                _ => {
                    eprintln!("join-lab: diverge takes two engines (--engine program,metal)");
                    std::process::exit(2);
                }
            };
            for w in waves(target, only) {
                eprintln!("join-lab: {} on {} and {}", w.name(), a.name(), b.name());
                match rerail_join_lab::first_divergence(&w, a, b) {
                    Ok(None) => println!("{:<16} agree", w.name()),
                    Ok(Some(d)) => println!("{:<16} {}", w.name(), d.line()),
                    Err(err) => println!("{:<16} FAILED {err}", w.name()),
                }
            }
        }
        "script" => {
            // Each launch's size script as each engine's load binds it: its addresses placed in their regions.
            for w in waves(target, only) {
                for e in engines(&args) {
                    // After its first `--upto K` dispatches run (none by default).
                    let upto = flag(&args, "--upto").and_then(|k| k.parse::<usize>().ok());
                    let loaded = w.load_on(e).and_then(|mut l| {
                        if let Some(k) = upto {
                            l.upto(k);
                            l.run_on(e)?;
                        }
                        Ok(l)
                    });
                    match loaded {
                        Ok(l) => {
                            println!("{} on {}{}", w.name(), e.name(), upto.map_or(String::new(), |k| format!(" after {k} dispatches")));
                            for op in l.script_ops() {
                                println!("  {op}");
                            }
                            // Its phases' size items (the script's runs of ops).
                            for p in 0..w.args[0] {
                                for (kind, x, group, seg) in l.phase_items(p).into_iter().filter(|i| i.0 == "size") {
                                    println!("  phase {p} item: {kind} x {x} group {group} seg {seg}");
                                }
                            }
                        }
                        Err(err) => println!("{} on {}: FAILED {err}", w.name(), e.name()),
                    }
                }
            }
        }
        "profile" => {
            let repeat = flag(&args, "--repeat").and_then(|n| n.parse().ok()).unwrap_or(5usize).max(1);
            let out = flag(&args, "-o").unwrap_or("results.tsv");
            // On one core where the host binds a thread to one (`--core N`, Linux; the last core by default).
            let core = flag(&args, "--core").and_then(|n| n.parse().ok()).unwrap_or_else(|| std::thread::available_parallelism().map_or(0, |n| n.get() - 1));
            let pinned = rerail_join_lab::counters::pin(core);
            let counted = rerail_join_lab::counters::Counters::open().is_some();
            eprintln!("join-lab: {}, {}", if pinned { format!("pinned to core {core}") } else { "not pinned (the host binds no thread to a core)".to_owned() }, if counted { "the thread's instructions and cycles counted" } else { "no counters (the host gives none)" });
            let mut rows = vec![rerail_join_lab::Profile::header()];
            // Each engine's stacks, sampled each millisecond its runs take (`--sample`).
            let sampler = args.iter().any(|a| a == "--sample").then(|| rerail_join_lab::sample::Sampler::start(std::time::Duration::from_millis(1))).and_then(|s| s.map_err(|why| eprintln!("join-lab: --sample: {why}")).ok());
            let engines = engines(&args);
            for w in waves(target, only) {
                for (i, &e) in engines.iter().enumerate() {
                    eprintln!("join-lab: {} on {}", w.name(), e.name());
                    if e == Engine::Class
                        && let Err(why) = w.classes()
                    {
                        println!("{:<16} {:<8} none   {why}", w.name(), e.name());
                        continue;
                    }
                    match rerail_join_lab::profile(&w, e, repeat, sampler.as_ref().map(|s| (s, i as u32 + 1))) {
                        Ok(p) => {
                            println!("{}", p.line());
                            rows.push(p.tsv());
                        }
                        Err(err) => println!("{:<16} {:<8} FAILED {err}", w.name(), e.name()),
                    }
                }
            }
            // The process's peak resident memory (every engine's replays in it): bytes on macOS, KiB on Linux.
            // SAFETY: a query of this process's usage into a zeroed struct of ours.
            let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
            if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } == 0 {
                let peak = usage.ru_maxrss as u64 * if cfg!(target_vendor = "apple") { 1 } else { 1024 };
                rows.push(format!("# peak_rss {peak}"));
            }
            if let Err(e) = std::fs::write(out, rows.join("\n") + "\n") {
                eprintln!("join-lab: {out}: {e}");
                std::process::exit(1);
            }
            if let Some(s) = sampler {
                let samples = s.finish();
                match rerail_join_lab::sample::fold(&samples) {
                    Ok(by) => {
                        for (i, e) in engines.iter().enumerate() {
                            let folded: String = by.get(&(i as u32 + 1)).map_or(String::new(), |v| v.iter().map(|(p, n)| format!("{p} {n}\n")).collect());
                            let path = Path::new(out).with_extension(format!("{}.folded", e.name()));
                            eprintln!("join-lab: {} samples of {} in {}", folded.lines().map(|l| l.rsplit(' ').next().and_then(|n| n.parse::<u64>().ok()).unwrap_or(0)).sum::<u64>(), e.name(), path.display());
                            if let Err(err) = std::fs::write(&path, folded) {
                                eprintln!("join-lab: {}: {err}", path.display());
                            }
                        }
                    }
                    Err(why) => eprintln!("join-lab: --sample: {why}"),
                }
            }
        }
        "words" => {
            // join-lab words <capture> --only <launch> <lo> <hi>: the words of its regions (as loaded) in lo..hi.
            let hex = |i: usize| args.get(i).and_then(|h| u64::from_str_radix(h.trim_start_matches("0x"), 16).ok()).unwrap_or_else(|| usage());
            let (lo, hi) = (hex(args.len() - 2), hex(args.len() - 1));
            for w in waves(target, only) {
                let name = w.name();
                match w.load() {
                    Ok(l) => {
                        if let Some(of) = flag(&args, "--relocs") {
                            for r in l.relocations().unwrap_or_default() {
                                if of.split(',').any(|n| r.starts_with(&format!("{n} "))) {
                                    println!("{name} reloc {r}");
                                }
                            }
                        }
                        for a in l.atoms() {
                            println!("{name} {a}");
                        }
                        match l.views() {
                            Ok(v) => {
                                for b in v {
                                    println!("{name} views: {b}");
                                }
                            }
                            Err(e) => println!("{name} views FAILED {e}"),
                        }
                        for (r, b, n) in l.placed() {
                            if (b as u64) < hi + (1 << 30) && (b + n) as u64 + (1 << 30) > lo {
                                println!("{name} placed {r} at {b:#x} len {n:#x} (to {:#x})", b + n);
                            }
                        }
                        for (r, o, v) in l.words_in(lo, hi) {
                            println!("{name} {r} {o} {v:#x}");
                        }
                    }
                    Err(e) => println!("{name} FAILED {e}"),
                }
            }
        }
        "page" => {
            let out = flag(&args, "-o").unwrap_or("page.html");
            // The engines' stacks beside the results (`profile --sample`), each its engine's.
            let folded = |engine: &str| std::fs::read_to_string(Path::new(target).with_extension(format!("{engine}.folded"))).ok();
            let html = std::fs::read_to_string(target).map_err(|e| e.to_string()).and_then(|t| rerail_join_lab::page::write(&t, catalogue(&args).as_ref(), &folded));
            match html {
                Ok(h) => {
                    if let Err(e) = std::fs::write(out, h) {
                        eprintln!("join-lab: {out}: {e}");
                        std::process::exit(1);
                    }
                }
                Err(e) => {
                    eprintln!("join-lab: {target}: {e}");
                    std::process::exit(1);
                }
            }
        }
        _ => usage(),
    }
}
