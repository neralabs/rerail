//! The parser over a list of files, many times (for a profiler): `bench <list> <seconds> [lex]`.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let files: Vec<Vec<u8>> = std::fs::read_to_string(&args[1]).unwrap().lines().filter_map(|p| std::fs::read(p).ok()).collect();
    let secs: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10.0);
    let lex_only = args.get(3).is_some_and(|a| a == "lex");
    let t0 = std::time::Instant::now();
    let (mut n, mut passes) = (0usize, 0);
    while t0.elapsed().as_secs_f64() < secs {
        for f in &files {
            n += if lex_only { rerail_python_lean::token_count(f).unwrap_or(0) } else { rerail_python_lean::tape(f).map_or(0, |t| t.code.len()) };
        }
        passes += 1;
    }
    let bytes: usize = files.iter().map(Vec::len).sum();
    eprintln!("{passes} passes, {:.2} ns a byte ({n})", t0.elapsed().as_secs_f64() * 1e9 / (bytes * passes) as f64);
}
