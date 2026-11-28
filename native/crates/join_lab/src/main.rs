//! `join-lab replay <capture> [--repeat N]`: each launch captured under `<capture>` (`RERAIL_JOIN_CAPTURE`'s
//! directory) replayed through the join program's Rust, timed, and held to what the engine's launch left.

use rerail_join_lab::Wave;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some("replay"), Some(capture)) = (args.first().map(String::as_str), args.get(1)) else {
        eprintln!("usage: join-lab replay <capture> [--repeat N]");
        std::process::exit(2);
    };
    let repeat = args.iter().position(|a| a == "--repeat").and_then(|i| args.get(i + 1)).and_then(|n| n.parse().ok()).unwrap_or(1usize);
    let waves = Wave::all(std::path::Path::new(capture)).unwrap_or_else(|e| {
        eprintln!("join-lab: {capture}: {e}");
        std::process::exit(1);
    });
    let mut failed = 0;
    println!("{:<14} {:>4} {:>9} {:>10} {:>10}  {:<8} shapes", "launch", "disp", "groups", "best ms", "regions", "after");
    for w in &waves {
        let name = w.dir.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let mut best = f64::INFINITY;
        let mut last = None;
        for _ in 0..repeat.max(1) {
            let mut l = match w.load() {
                Ok(l) => l,
                Err(e) => {
                    println!("{name:<14} not loaded: {e}");
                    failed += 1;
                    break;
                }
            };
            let ran = l.run();
            best = best.min(ran.seconds);
            last = Some((ran, l.compare()));
        }
        let Some((ran, diffs)) = last else { continue };
        let verdict = match &diffs {
            Ok(d) if d.iter().all(|d| d.differ == 0) => "same".to_owned(),
            Ok(d) => {
                failed += 1;
                d.iter().filter(|d| d.differ > 0).map(|d| format!("{}:{}", d.region, d.differ)).collect::<Vec<_>>().join(",")
            }
            Err(e) => {
                failed += 1;
                format!("{e}")
            }
        };
        let shapes: Vec<&str> = w.labels.iter().map(|l| l.shape.as_str()).collect();
        println!("{name:<14} {:>4} {:>9} {:>10.3} {:>10}  {:<8} {}", ran.dispatches, ran.groups, best * 1e3, w.regions.len(), verdict, shapes.join(" "));
    }
    if failed > 0 {
        std::process::exit(1);
    }
}
