//! `join-lab`: the launches captured under a directory (`RERAIL_JOIN_CAPTURE`'s), apart from the engine.
//!
//! - `join-lab list <capture>`: each launch, its shapes by work, its regions and bytes.
//! - `join-lab run <capture> [--engine program] [--only <text>]`: each launch replayed on each engine, held to what
//!   the engine's launch left; an engine that leaves otherwise is an error naming its first differing region.
//! - `join-lab profile <capture> [--engine program] [--repeat N] [--only <text>] [-o results.tsv]`: each launch run
//!   warm N times (loaded afresh each time, the load not timed): its best and median wall time and the thread's CPU
//!   time, written as a results table.
//! - `join-lab page <results.tsv> -o page.html`: the page of a profile's results.

use rerail_join_lab::{Engine, Wave};
use std::path::Path;

fn usage() -> ! {
    eprintln!("usage: join-lab list <capture>\n       join-lab run <capture> [--engine program] [--only <text>]\n       join-lab profile <capture> [--engine program] [--repeat N] [--only <text>] [-o results.tsv]\n       join-lab page <results.tsv> -o page.html");
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

fn engines(args: &[String]) -> Vec<Engine> {
    flag(args, "--engine").unwrap_or("program").split(',').map(|e| Engine::named(e).unwrap_or_else(|why| {
        eprintln!("join-lab: {why}");
        std::process::exit(2);
    })).collect()
}

fn main() {
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
        }
        "run" => {
            let mut failed = 0;
            for w in waves(target, only) {
                for e in engines(&args) {
                    // Its name first: a replay that faults names its launch.
                    eprintln!("join-lab: {} on {}", w.name(), e.name());
                    match rerail_join_lab::replay(&w, e) {
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
        "profile" => {
            let repeat = flag(&args, "--repeat").and_then(|n| n.parse().ok()).unwrap_or(5usize).max(1);
            let out = flag(&args, "-o").unwrap_or("results.tsv");
            let mut rows = vec![rerail_join_lab::Profile::header()];
            for w in waves(target, only) {
                for e in engines(&args) {
                    eprintln!("join-lab: {} on {}", w.name(), e.name());
                    match rerail_join_lab::profile(&w, e, repeat) {
                        Ok(p) => {
                            println!("{}", p.line());
                            rows.push(p.tsv());
                        }
                        Err(err) => println!("{:<16} {:<8} FAILED {err}", w.name(), e.name()),
                    }
                }
            }
            if let Err(e) = std::fs::write(out, rows.join("\n") + "\n") {
                eprintln!("join-lab: {out}: {e}");
                std::process::exit(1);
            }
        }
        "page" => {
            let out = flag(&args, "-o").unwrap_or("page.html");
            let html = std::fs::read_to_string(target).map_err(|e| e.to_string()).and_then(|t| rerail_join_lab::page::write(&t));
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
