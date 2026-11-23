//! The lean parser's speed, as `rerail-rr-aot`'s `rr-aot-speed` measures the parser as Rust: over every `.rerail` file
//! under a folder, its lexer alone (`token_count`) and the whole (`tape`), the least wall time of `RR_ROUNDS` rounds
//! (7), in turns. So the two are read side by side, the same files and clock:
//!
//! ```text
//! nice -n 19 cargo run --release -p rerail-rr-syntax --example lean_speed -- ..
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
    let bytes: usize = files.iter().map(String::len).sum();
    let time = |f: &dyn Fn() -> usize| {
        let t = Instant::now();
        std::hint::black_box(f());
        t.elapsed()
    };
    let lex = || files.iter().map(|t| rerail_rr_syntax::lean::token_count(t)).sum::<usize>();
    let tape = || files.iter().map(|t| rerail_rr_syntax::lean::tape(t).len()).sum::<usize>();
    let mut best = [Duration::MAX; 2];
    for _ in 0..rounds {
        best[0] = best[0].min(time(&lex));
        best[1] = best[1].min(time(&tape));
    }
    let mb = bytes as f64 / 1e6;
    let row = |name: &str, d: Duration| println!("  {name:<44} {:>9.3} ms  {:>8.1} MB/s  {:>6.2} ns/byte", d.as_secs_f64() * 1e3, mb / d.as_secs_f64(), d.as_secs_f64() * 1e9 / bytes as f64);
    println!("{} files, {mb:.2} MB, {} tokens; the least wall time of {rounds} rounds", files.len(), lex());
    row("lean: its lexer (token_count)", best[0]);
    row("lean: its parse (tape - lexer)", best[1].saturating_sub(best[0]));
    row("lean: whole (tape)", best[1]);
}
