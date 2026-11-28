//! The lexer of strata (`rerail_python_lean::strata`, decision 0698 item 93) against the lean lexer, over the `.py`
//! files under a directory: `compare_strata <dir> [seconds a side] [lean | strata | strata-b | 1 | 2 | 3 | 2b | 3b | chunked]`.
//!
//! First each file's tokens and extras compared (the same, either refusing, or the first token that differs), then
//! both lexers timed over the files both read the same, each many times on one thread. With `lean` or `strata` only
//! that lexer runs (with 1, 2 or 3 the strata up to that one alone, `b` for stratum 2 by variant B), for the timing
//! alone: `/usr/bin/time -l` then gives its instructions and cycles.
use rerail_python_lean::strata::{self, Check, Variant};

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
    let strata_b = |b: &[u8]| strata::token_count_with(b, Variant::B).unwrap_or(0);
    if only == Some("chunked") {
        // From the middle: each file in chunks of 1, 2, 3 and 16 blocks, each from the one before's exit, against the
        // serial lexer (variant B's).
        let sizes: Vec<usize> = std::env::var("CHUNK_BLOCKS").map_or(vec![1, 2, 3, 16], |v| v.split(',').filter_map(|x| x.parse().ok()).collect());
        for blocks in sizes {
            let t0 = std::time::Instant::now();
            let (mut same, mut not_b, mut differ) = (0, 0, 0);
            for (p, b) in &all {
                match strata::middle::chunked_is_serial(b, blocks) {
                    Ok(true) => same += 1,
                    Ok(false) => not_b += 1,
                    Err(e) => {
                        differ += 1;
                        if differ <= 3 {
                            println!("{} :: {}", p.strip_prefix(dir).unwrap_or(p).display(), e.lines().skip(1).collect::<Vec<_>>().join("\n"));
                        }
                    }
                }
            }
            let (mut ok, mut bad) = (0, 0);
            for (p, b) in &all {
                match strata::middle::entries_are_the_folds(b, blocks, &strata::middle::in_order) {
                    Some(Ok(())) => ok += 1,
                    Some(Err(e)) => {
                        bad += 1;
                        if bad <= 3 {
                            println!("{} :: {e}", p.strip_prefix(dir).unwrap_or(p).display());
                        }
                    }
                    None => {}
                }
            }
            let (mut m_ok, mut m_unknown, mut m_bad) = (0, 0, 0);
            for (p, b) in &all {
                match strata::middle::middle_is_serial(b, blocks, &strata::middle::in_order) {
                    Ok(None) => m_ok += 1,
                    Ok(Some(m)) => {
                        m_unknown += 1;
                        if m_unknown <= 3 {
                            println!("{} :: not entered: {m:?}", p.strip_prefix(dir).unwrap_or(p).display());
                        }
                    }
                    Err(e) => {
                        m_bad += 1;
                        if m_bad <= 3 {
                            println!("{} :: from the middle: {e}", p.strip_prefix(dir).unwrap_or(p).display());
                        }
                    }
                }
            }
            println!("chunks of {blocks} blocks: from the middle, every pass on its own: {m_ok} the serial lexer's, {m_unknown} a chunk not entered, {m_bad} wrong");
            println!("chunks of {blocks} blocks: the scan of summaries gives each chunk its entry in {ok} files, not in {bad}");
            println!("chunks of {blocks} blocks: {same} the serial lexer's records, {differ} not, {not_b} read by A (B's reading wrong); {:.1} s", t0.elapsed().as_secs_f64());
        }
        return;
    }
    if let Some(side) = only {
        let fs: Vec<&[u8]> = all.iter().map(|(_, b)| b.as_slice()).filter(|b| strata::check(b) == Check::Same).collect();
        let bytes: usize = fs.iter().map(|b| b.len()).sum();
        let (ns, n, passes) = match side {
            "lean" => time(&fs, secs, lean),
            "1" | "2" | "3" => {
                let k: u8 = side.parse().expect("a stratum");
                time(&fs, secs, |b| strata::upto(b, k, Variant::A))
            }
            "2b" | "3b" => {
                let k: u8 = side[..1].parse().expect("a stratum");
                time(&fs, secs, |b| strata::upto(b, k, Variant::B))
            }
            "strata-b" => time(&fs, secs, strata_b),
            _ => time(&fs, secs, strata),
        };
        eprintln!("{side}: {} files, {bytes} bytes, {passes} passes: {ns:.3} ns/B   [{n}]", fs.len());
        return;
    }
    // Variant B first: its checks, and how many files it reads itself.
    let (mut b_same, mut b_own, mut b_other) = (0, 0, 0);
    for (p, b) in &all {
        let name = p.strip_prefix(dir).unwrap_or(p).display();
        b_own += usize::from(strata::b_reads(b));
        match (strata::check_with(b, Variant::B), strata::check(b)) {
            (Check::Same, _) => b_same += 1,
            (x, y) if x == y => {}
            (Check::StrataRefuses(_), Check::StrataRefuses(_)) => {}
            (x, y) => {
                b_other += 1;
                println!("B {x:?}, A {y:?} :: {name}");
            }
        }
    }
    println!("variant B: {b_same} the same, {b_other} not as A, read itself {b_own} of {}", all.len());
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
    // A pass of each in turn, so that both lexers meet the machine's same load.
    let (mut tl, mut ts, mut tb, mut passes, mut sum) = (0f64, 0f64, 0f64, 0usize, 0usize);
    let t0 = std::time::Instant::now();
    while passes == 0 || t0.elapsed().as_secs_f64() < 3.0 * secs {
        let (l, a, _) = time(&same_files, 0.0, lean);
        let (s, b, _) = time(&same_files, 0.0, strata);
        let (x, c, _) = time(&same_files, 0.0, strata_b);
        (tl, ts, tb, passes, sum) = (tl + l, ts + s, tb + x, passes + 1, sum.wrapping_add(a ^ b ^ c));
    }
    let (l, s, x) = (tl / passes as f64, ts / passes as f64, tb / passes as f64);
    println!("lexers over the {} files the same ({bytes} bytes), {passes} passes each in turn: lean {l:.3} ns/B, strata A {s:.3} ns/B ({:.2}x), B {x:.3} ns/B ({:.2}x)   [{sum}]", same_files.len(), l / s, l / x);
}
