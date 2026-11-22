//! The parser as Rust's speed in its product build (no `test-hooks`: a binary takes no dev-dependency, so tier 1 has
//! no walk and no checks): over every `.rerail` file under a folder (the repository's, as `rerail-rr-parser`'s
//! `corpus`), tier 1 alone (`lex`) and the whole (`parse`), the least wall time of `RR_ROUNDS` rounds (7), in turns,
//! the buffers padded before. Build it optimized and run it at nice 19:
//!
//! ```text
//! nice -n 19 cargo run --profile profiling -p rerail-rr-aot --bin rr-aot-speed -- ..
//! ```

use std::time::{Duration, Instant};

fn main() {
    let root = std::env::args().nth(1).unwrap_or_else(|| "..".to_owned());
    let rounds: usize = std::env::var("RR_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(7);
    let (mut files, mut work) = (Vec::new(), vec![std::path::PathBuf::from(root)]);
    while let Some(dir) = work.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let skip = p.file_name().is_some_and(|n| [".git", "target", "node_modules"].contains(&n.to_string_lossy().as_ref()));
            if p.is_dir() && !skip {
                work.push(p);
            } else if p.extension().is_some_and(|x| x == "rerail") {
                files.extend(std::fs::read_to_string(&p).ok());
            }
        }
    }
    files.sort();
    let bufs: Vec<Vec<u8>> = files.iter().map(|t| rerail_rr_aot::padded(t.as_bytes())).collect();
    let bytes: usize = files.iter().map(String::len).sum();
    let time = |f: &dyn Fn() -> usize| {
        let t = Instant::now();
        std::hint::black_box(f());
        t.elapsed()
    };
    let lex = || bufs.iter().zip(&files).map(|(b, t)| rerail_rr_aot::lex(b, t.len()).starts.len()).sum::<usize>();
    let parse = || bufs.iter().zip(&files).map(|(b, t)| rerail_rr_aot::parse(b, t.len()).map_or(0, |c| c.keys.len())).sum::<usize>();
    let mut best = [Duration::MAX; 2];
    for _ in 0..rounds {
        best[0] = best[0].min(time(&lex));
        best[1] = best[1].min(time(&parse));
    }
    // The corpus as one file, for what a file costs apart from its bytes.
    let all: String = files.concat();
    let one = rerail_rr_aot::padded(all.as_bytes());
    let mut whole = Duration::MAX;
    for _ in 0..rounds {
        whole = whole.min(time(&|| rerail_rr_aot::lex(&one, all.len()).starts.len()));
    }
    let refused = bufs.iter().zip(&files).filter(|(b, t)| rerail_rr_aot::parse(b, t.len()).is_err()).count();
    let mb = bytes as f64 / 1e6;
    let row = |name: &str, d: Duration| println!("  {name:<44} {:>9.3} ms  {:>8.1} MB/s  {:>6.2} ns/byte", d.as_secs_f64() * 1e3, mb / d.as_secs_f64(), d.as_secs_f64() * 1e9 / bytes as f64);
    println!("{} files, {mb:.2} MB, {refused} refused; the least wall time of {rounds} rounds", files.len());
    row("tier 1 (lex)", best[0]);
    row("tier 2 (parse - lex)", best[1].saturating_sub(best[0]));
    row("whole (parse)", best[1]);
    row("tier 1 (lex), the corpus as one file", whole);
}
