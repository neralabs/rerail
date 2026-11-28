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
    if only == Some("starts") || only == Some("strings") || only == Some("extras") {
        // The walk's token starts by masks against the serial walk's, block by block: in code, or in strings.
        let (mut same, mut rare, mut differ, mut blocks) = (0, 0, 0, 0);
        let oracle = match only {
            Some("starts") => strata::starts::starts_are_the_walks,
            Some("strings") => strata::starts::string_starts_are_the_walks,
            _ => strata::starts::extras_are_the_walks,
        };
        for (p, b) in &all {
            if let Some((s, r, first)) = oracle(b) {
                same += s;
                rare += r;
                blocks += b.len().div_ceil(64);
                if let Some(e) = first {
                    differ += 1;
                    if differ <= 8 {
                        println!("{} :: {e}", p.strip_prefix(dir).unwrap_or(p).display());
                    }
                }
            }
        }
        println!("token starts by masks ({}): {blocks} blocks, {same} the walk's, {rare} rare, {differ} files with a block that differs", only.unwrap_or_default());
        return;
    }
    if only == Some("layout") {
        // The walk's summaries by masks against the walk's, chunk by chunk, at each chunk size.
        for blocks in [1, 3, 16] {
            let (mut same, mut unknown, mut differ, mut files) = (0, 0, 0, 0);
            for (p, b) in &all {
                if let Some((s, u, first)) = strata::middle::layout_is_the_walks(b, blocks) {
                    files += 1;
                    same += s;
                    unknown += u;
                    if let Some(e) = first {
                        differ += 1;
                        if differ <= 6 {
                            println!("{} :: {e}", p.strip_prefix(dir).unwrap_or(p).display());
                        }
                    }
                }
            }
            println!("chunks of {blocks} blocks: walk summaries by masks: {files} files, {same} lanes the walk's, {unknown} not known, {differ} files with a lane that differs");
        }
        return;
    }
    if only == Some("tops") {
        // The walk's loop tops by masks against the walk's, at each block's start.
        let (mut same, mut differ, mut files) = (0, 0, 0);
        for (p, b) in &all {
            if let Some((s, first)) = strata::middle::tops_are_the_walks(b) {
                files += 1;
                same += s;
                if let Some(e) = first {
                    differ += 1;
                    if differ <= 8 {
                        println!("{} :: {e}", p.strip_prefix(dir).unwrap_or(p).display());
                    }
                }
            }
        }
        println!("loop tops by masks: {files} files, {same} blocks the walk's, {differ} files with a block that differs");
        return;
    }
    if only == Some("passes") {
        // From the middle, its passes: with no STAGE, the counts per chunk size (chunks, lanes not known, waits, by a
        // lane or lexed again); with STAGE (1 to 6, `strata::middle::Stage`; 0 the setup alone, 7 the serial lexer),
        // PASSES passes over the files lexed from the middle at CHUNK_BLOCKS, for `/usr/bin/time -l`'s cycles.
        use strata::middle::{MiddleCounts, Stage, lex_middle_to};
        let env = |k: &str, d: usize| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
        let stage = std::env::var("STAGE").ok().and_then(|v| v.parse::<usize>().ok());
        let Some(stage) = stage else {
            println!("| Chunk size (blocks) | Chunks | Lanes not known | Waits | By the lane before | Lexed again |");
            println!("|---|---|---|---|---|---|");
            for blocks in [1, 2, 3, 16, 256] {
                let mut c = MiddleCounts::default();
                for (_, b) in &all {
                    let _ = lex_middle_to(b, blocks, &strata::middle::in_order, Stage::Lexed, &mut c);
                }
                println!("| {blocks} | {} | {} | {} | {} | {} |", c.chunks, c.unknown_lanes, c.waits, c.by_lane, c.lexed_again);
            }
            return;
        };
        let blocks = env("CHUNK_BLOCKS", 16);
        let passes = env("PASSES", 20);
        let fs: Vec<&[u8]> = all.iter().map(|(_, b)| b.as_slice()).filter(|b| matches!(strata::middle::lex_middle(b, blocks, &strata::middle::in_order), Ok(strata::middle::Middle::Lexed(_)))).collect();
        let bytes: usize = fs.iter().map(|b| b.len()).sum();
        let stages = [Stage::StrataSummaries, Stage::StrataScan, Stage::WalkInputs, Stage::WalkSummaries, Stage::WalkScan, Stage::Lexed];
        let t0 = std::time::Instant::now();
        let mut sum = 0usize;
        for _ in 0..if stage == 0 { 0 } else { passes } {
            for b in &fs {
                sum = sum.wrapping_add(match stage {
                    7 => strata_b(b),
                    s => match lex_middle_to(b, blocks, &strata::middle::in_order, stages[s - 1], &mut MiddleCounts::default()) {
                        Ok(strata::middle::Middle::Stopped(x)) => x,
                        Ok(strata::middle::Middle::Lexed(l)) => l.toks.len(),
                        _ => 1,
                    },
                });
            }
        }
        let ns = t0.elapsed().as_secs_f64() * 1e9 / (bytes * passes.max(1)) as f64;
        println!("stage {stage}, {blocks} blocks a chunk: {} files, {bytes} bytes, {passes} passes, {ns:.3} ns/B   [{sum}]", fs.len());
        return;
    }
    if only == Some("rare") {
        // The blocks whose starts are rare, by cause.
        let (mut code, mut strs, mut blocks) = ([0usize; 6], [0usize; 8], 0);
        for (_, b) in &all {
            if let Some((c, q)) = strata::starts::rare_causes(b) {
                blocks += b.len().div_ceil(64);
                code.iter_mut().zip(c).for_each(|(x, y)| *x += y);
                strs.iter_mut().zip(q).for_each(|(x, y)| *x += y);
            }
        }
        println!("{blocks} blocks; rare in code, by cause:");
        for (n, c) in strata::starts::CODE_NAMES.iter().zip(code) {
            println!("  {n:<14} {c:>6}");
        }
        println!("rare in strings, by cause:");
        for (n, c) in strata::starts::STR_NAMES.iter().zip(strs) {
            println!("  {n:<14} {c:>6}");
        }
        return;
    }
    if only == Some("why") {
        // Why pass 1 by masks does not know lanes: per reason, the lanes among its reasons, and those it alone marks.
        for blocks in [1, 3, 16] {
            let (mut lanes, mut any, mut alone) = (0, [0usize; 13], [0usize; 13]);
            for (_, b) in &all {
                if let Some((l, a, o, f)) = strata::middle::walk_why_counts(b, blocks) {
                    if blocks == 1 {
                        for (i, at) in f.iter().enumerate() {
                            if let Some(bi) = at
                                && any[i] < 4
                                && std::env::var("WHY_SHOW").is_ok_and(|v| v == strata::middle::WHY_NAMES[i])
                            {
                                println!("{} at block {bi}: {:?}", strata::middle::WHY_NAMES[i], String::from_utf8_lossy(&b[64 * bi..(64 * bi + 192).min(b.len())]));
                            }
                        }
                    }
                    lanes += l;
                    for i in 0..13 {
                        any[i] += a[i];
                        alone[i] += o[i];
                    }
                }
            }
            println!("chunks of {blocks} blocks: {lanes} lanes; not known, by reason (among its reasons / its only one):");
            for i in 0..13 {
                if any[i] > 0 {
                    println!("  {:<13} {:>7} / {:>7}", strata::middle::WHY_NAMES[i], any[i], alone[i]);
                }
            }
        }
        return;
    }
    if only == Some("scan") {
        // Pass 3 of the walk in fixed-size lanes against the scan by `Sym`, over the summaries by masks.
        for blocks in [1, 3, 16] {
            let (mut same, mut waits, mut bad) = (0, 0, 0);
            for (p, b) in &all {
                if let Some((s, w, first)) = strata::middle::walk_scan_is_syms(b, blocks, &strata::middle::in_order) {
                    same += s;
                    waits += w;
                    if let Some(e) = first {
                        bad += 1;
                        if bad <= 3 {
                            println!("{} :: {e}", p.strip_prefix(dir).unwrap_or(p).display());
                        }
                    }
                }
            }
            println!("chunks of {blocks} blocks: the walk's scan in fixed size: {same} entries the scan by Sym's, {waits} it does not know (the chunk waits), {bad} files with one that differs");
        }
        return;
    }
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
            let mut s_bad = 0;
            for (p, b) in &all {
                if let Err(e) = strata::middle::superposed_is_per_lane(b, blocks) {
                    s_bad += 1;
                    if s_bad <= 3 {
                        println!("{} :: pass 1 superposed: {e}", p.strip_prefix(dir).unwrap_or(p).display());
                    }
                }
            }
            println!("chunks of {blocks} blocks: pass 1 superposed is B from every lane in {} files, not in {s_bad}", all.len() - s_bad);
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
