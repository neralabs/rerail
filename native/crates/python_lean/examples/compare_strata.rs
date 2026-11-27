//! The lexer of strata (`rerail_python_lean::strata`, decision 0698 item 93) against the lean lexer, over the `.py`
//! files under a directory: `compare_strata <dir> [seconds a side] [lean | strata | 1 | 2 | 3]`.
//!
//! First each file's tokens and extras compared (the same, either refusing, or the first token that differs), then
//! both lexers timed over the files both read the same, each many times on one thread. With `lean` or `strata` only
//! that lexer runs (with 1, 2 or 3 the strata up to that one alone), for the timing alone: `/usr/bin/time -l` then gives its instructions and cycles.
use rerail_python_lean::strata::{self, Check};

fn files(dir: &std::path::Path, out: &mut Vec<(std::path::PathBuf, Vec<u8>)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<_> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            files(&p, out);
        } else if p.extension().is_some_and(|e| e == "py")
            && let Ok(b) = std::fs::read(&p)
        {
            out.push((p, b));
        }
    }
}

/// `f` over every file, in passes, for `secs`: ns a byte, and a sum that keeps the work.
fn time(fs: &[&[u8]], secs: f64, f: impl Fn(&[u8]) -> usize) -> (f64, usize, usize) {
    let bytes: usize = fs.iter().map(|b| b.len()).sum();
    let t0 = std::time::Instant::now();
    let (mut n, mut passes) = (0usize, 0usize);
    while passes == 0 || t0.elapsed().as_secs_f64() < secs {
        for b in fs {
            n = n.wrapping_add(f(b));
        }
        passes += 1;
    }
    (t0.elapsed().as_secs_f64() * 1e9 / (bytes * passes) as f64, n, passes)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = std::path::Path::new(&args[1]);
    let secs: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(3.0);
    let only = args.get(3).map(String::as_str);
    let mut all = Vec::new();
    files(dir, &mut all);
    let lean = |b: &[u8]| rerail_python_lean::token_count(b).unwrap_or(0);
    let strata = |b: &[u8]| strata::token_count(b).unwrap_or(0);
    if let Some(side) = only {
        let fs: Vec<&[u8]> = all.iter().map(|(_, b)| b.as_slice()).filter(|b| strata::check(b) == Check::Same).collect();
        let bytes: usize = fs.iter().map(|b| b.len()).sum();
        let (ns, n, passes) = match side {
            "lean" => time(&fs, secs, lean),
            "1" | "2" | "3" => {
                let k: u8 = side.parse().expect("a stratum");
                time(&fs, secs, |b| strata::upto(b, k))
            }
            _ => time(&fs, secs, strata),
        };
        eprintln!("{side}: {} files, {bytes} bytes, {passes} passes: {ns:.3} ns/B   [{n}]", fs.len());
        return;
    }
    let (mut same, mut both, mut lean_refuses, mut strata_refuses, mut differ) = (0, 0, 0, 0, 0);
    let mut same_files = Vec::new();
    for (p, b) in &all {
        let name = p.strip_prefix(dir).unwrap_or(p).display();
        match strata::check(b) {
            Check::Same => {
                same += 1;
                same_files.push(b.as_slice());
            }
            Check::BothRefuse => both += 1,
            Check::LeanRefuses(at) => {
                lean_refuses += 1;
                println!("lean refuses (byte {at}) :: {name}");
            }
            Check::StrataRefuses(at) => {
                strata_refuses += 1;
                let line = b[..(at as usize).min(b.len())].iter().filter(|&&c| c == b'\n').count() + 1;
                println!("strata refuse (byte {at}, line {line}) :: {name}");
            }
            Check::Differ(a, s) => {
                differ += 1;
                println!("differs :: {name}\n  lean:\n{a}\n  strata:\n{s}");
            }
        }
    }
    println!("{} files: {same} the same, {differ} differ, {strata_refuses} the strata refuse, {lean_refuses} the lean lexer refuses, {both} both refuse", all.len());
    let bytes: usize = same_files.iter().map(|b| b.len()).sum();
    let (l, a, _) = time(&same_files, secs, lean);
    let (s, b, _) = time(&same_files, secs, strata);
    println!("lexers over the {} files the same ({bytes} bytes): lean {l:.3} ns/B, strata {s:.3} ns/B ({:.2}x)   [{a} {b}]", same_files.len(), l / s);
}
