//! The parser as Rust against the lean parser, over the `.py` files under a directory, each side many times:
//! `compare <dir> <seconds a side>`. Only the files both parse are timed. Each side's lexer alone (the AOT's tier 1,
//! the lean parser's tokens), then its whole parse (to the format's columns, to the lean tape), on one thread.
fn files(dir: &std::path::Path, out: &mut Vec<Vec<u8>>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<_> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            files(&p, out);
        } else if p.extension().is_some_and(|e| e == "py")
            && let Ok(b) = std::fs::read(&p)
        {
            out.push(b);
        }
    }
}

/// `f` over every file, in passes, for `secs`: ns a byte, and a sum that keeps the work.
fn time(fs: &[Vec<u8>], secs: f64, f: impl Fn(&[u8]) -> usize) -> (f64, usize) {
    let bytes: usize = fs.iter().map(Vec::len).sum();
    let t0 = std::time::Instant::now();
    let (mut n, mut passes) = (0usize, 0usize);
    while passes == 0 || t0.elapsed().as_secs_f64() < secs {
        for b in fs {
            n = n.wrapping_add(f(b));
        }
        passes += 1;
    }
    (t0.elapsed().as_secs_f64() * 1e9 / (bytes * passes) as f64, n)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let secs: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5.0);
    let mut all = Vec::new();
    files(std::path::Path::new(&args[1]), &mut all);
    let total = all.len();
    let both: Vec<Vec<u8>> = all.into_iter().filter(|b| rerail_python_aot::parse(b).is_ok() && rerail_python_lean::tape(b).is_ok()).collect();
    let bytes: usize = both.iter().map(Vec::len).sum();
    eprintln!("{} of {total} files both parse, {bytes} bytes", both.len());
    let (aot_lex, a) = time(&both, secs, |b| rerail_python_aot::lex(b).recs.len());
    let (lean_lex, b) = time(&both, secs, |b| rerail_python_lean::token_count(b).unwrap_or(0));
    let (aot, c) = time(&both, secs, |b| rerail_python_aot::parse(b).map_or(0, |t| t.code.len()));
    let (lean, d) = time(&both, secs, |b| rerail_python_lean::tape(b).map_or(0, |t| t.code.len()));
    eprintln!("lexer: aot {aot_lex:.2} ns/B, lean {lean_lex:.2} ns/B ({:.2}x)   [{a} {b}]", aot_lex / lean_lex);
    eprintln!("parse: aot {aot:.2} ns/B, lean {lean:.2} ns/B ({:.2}x)   [{c} {d}]", aot / lean);
}
