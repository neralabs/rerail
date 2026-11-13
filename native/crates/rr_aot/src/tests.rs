//! The ahead-of-time build against the JIT's: the same tapes, then each one's cost.

use rerail_rr_syntax as rr;

/// The repository's `.rr` files.
fn corpus() -> Vec<String> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let (mut files, mut work) = (Vec::new(), vec![root]);
    while let Some(dir) = work.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let skip = p.file_name().is_some_and(|n| [".git", "target", "node_modules"].contains(&n.to_string_lossy().as_ref()));
            if p.is_dir() && !skip {
                work.push(p);
            } else if p.extension().is_some_and(|x| x == "rr") {
                files.extend(std::fs::read_to_string(&p).ok());
            }
        }
    }
    files.sort();
    files
}

/// This thread's CPU time.
fn thread_cpu() -> std::time::Duration {
    let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: clock_gettime writes the structure.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    std::time::Duration::new(t.tv_sec as u64, t.tv_nsec as u32)
}

const BOTH: crate::Tiers = crate::Tiers { one: true, two: true };

/// Every corpus file: the ahead-of-time tier 1 writes the JIT's masks, positions and counts;
/// both tiers ahead of time write the JIT's tape, and refuse where it does.
#[test]
#[ignore = "the corpus: run by name"]
fn aot_tapes_are_the_jits() {
    let jit = rerail_rr_parser::compiled().expect("compiles");
    let aot = crate::aot(BOTH).expect("the ahead-of-time build");
    // The corpus, and the table's in-place cases (`rr_parser`'s `the_table_is_written_in_place`):
    // wraps, a drop then a reread.
    let mut files = corpus();
    for src in ["r(x) :- x = a - b - c - d.", "r(x) :- x = (a + b) * (c - d) - e / f.", "r(x) :- x = a /* c */ - b // d\n - c.", "r(n) :- n = min (a, b) : { s(a, b) }."] {
        files.push(src.to_string());
    }
    let (mut differ, mut tier1_differ) = (Vec::new(), Vec::new());
    let mut taken = 0;
    for (i, f) in files.iter().enumerate() {
        let mut a = rerail_compiler::native_cpu_block::Batch::new(&aot.parser.tier1, &[f.as_bytes()]);
        let mut j = rerail_compiler::native_cpu_block::Batch::new(&jit.parser.tier1, &[f.as_bytes()]);
        a.call(&aot.parser.tier1);
        j.call(&jit.parser.tier1);
        if a.out != j.out || a.positions != j.positions || a.counts != j.counts {
            tier1_differ.push(i);
        }
        match (aot.parser.columns(f.as_bytes()), jit.parser.columns(f.as_bytes())) {
            (Ok(a), Ok(j)) => {
                taken += 1;
                if a != j {
                    differ.push(i);
                }
            }
            (Err(_), Err(_)) => {}
            _ => differ.push(i),
        }
    }
    eprintln!("{} files, {taken} taken", files.len());
    assert_eq!(tier1_differ, Vec::<usize>::new(), "files whose tier 1 output is not the JIT's");
    assert_eq!(differ, Vec::<usize>::new(), "files whose tape is not the JIT's");
}

/// Each variant over the corpus files the grammar parses, the least of `RR_ROUNDS` rounds: CPU
/// time, instructions, cycles, and MB a second of CPU. Tier 1 and tier 2 as the JIT built them
/// and as rustc did, each alone on batches and tapes made before; the whole parse a file at a
/// time (`columns`: the batch, both tiers, the tape and its columns); the engine's parser
/// (`rr::tape`); the lean parser's lexer and whole.
#[test]
#[ignore = "a benchmark: run by name, on an optimized build"]
fn jit_against_aot() {
    let jit = rerail_rr_parser::compiled().expect("compiles");
    let aot = crate::aot(BOTH).expect("the ahead-of-time build");
    let rounds: usize = std::env::var("RR_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(7);
    let files = corpus();
    let ours: Vec<&String> = files.iter().filter(|f| jit.parser.columns(f.as_bytes()).is_ok()).collect();
    let nodes: usize = ours.iter().map(|f| rr::tape(f).len()).sum();
    let bytes: usize = ours.iter().map(|f| f.len()).sum();
    let batch = |p: &rerail_compiler::native_cpu_stream::Parser| -> Vec<_> {
        ours.iter()
            .map(|f| {
                let mut b = rerail_compiler::native_cpu_block::Batch::new(&p.tier1, &[f.as_bytes()]);
                b.call(&p.tier1);
                b
            })
            .collect()
    };
    let (mut bj, mut ba) = (batch(&jit.parser), batch(&aot.parser));
    let mut tapes: Vec<_> = bj.iter().map(|b| jit.parser.tape(b)).collect();
    let retired = rerail_compiler::native_cpu_codegen::retired;
    let names = ["tier 1 (JIT)", "tier 1 (AOT)", "tier 2 (JIT)", "tier 2 (AOT)", "whole (JIT)", "whole (AOT)", "Rust (rr::tape)", "lean: lexer", "lean: whole", "lean: the digest"];
    let mut best = vec![(std::time::Duration::MAX, u64::MAX, u64::MAX); names.len()];
    for _ in 0..rounds {
        let mut at = (thread_cpu(), retired());
        let mut lap = |k: usize, best: &mut Vec<(std::time::Duration, u64, u64)>| {
            let now = (thread_cpu(), retired());
            let (t, i, c) = (now.0 - at.0, now.1.0 - at.1.0, now.1.1 - at.1.1);
            best[k] = (best[k].0.min(t), best[k].1.min(i), best[k].2.min(c));
            at = (thread_cpu(), retired());
        };
        for b in &mut bj {
            b.call(&jit.parser.tier1);
        }
        lap(0, &mut best);
        for b in &mut ba {
            b.call(&aot.parser.tier1);
        }
        lap(1, &mut best);
        for (b, t) in bj.iter().zip(&mut tapes) {
            let _ = jit.parser.words(b, t);
        }
        lap(2, &mut best);
        for (b, t) in bj.iter().zip(&mut tapes) {
            let _ = aot.parser.words(b, t);
        }
        lap(3, &mut best);
        let mut n = 0;
        for f in &ours {
            n += jit.parser.columns(f.as_bytes()).map_or(0, |c| c.len());
        }
        lap(4, &mut best);
        for f in &ours {
            n += aot.parser.columns(f.as_bytes()).map_or(0, |c| c.len());
        }
        lap(5, &mut best);
        for f in &ours {
            n += rr::tape(f).len();
        }
        lap(6, &mut best);
        for f in &ours {
            n += rr::lean::token_count(f);
        }
        lap(7, &mut best);
        for f in &ours {
            n += rr::lean::tape(f).len();
        }
        lap(8, &mut best);
        // The tape header's digest alone: a part of the whole above.
        for f in &ours {
            n += usize::from(rerail_tape_format::digest(f.as_bytes())[0]);
        }
        std::hint::black_box(n);
        lap(9, &mut best);
    }
    let mb = bytes as f64 / 1e6;
    eprintln!("{} files the grammar parses, {mb:.2} MB, {nodes} nodes; the least of {rounds} rounds", ours.len());
    let row = |name: &str, (t, i, cy): (std::time::Duration, u64, u64)| {
        eprintln!("  {name:>22}: {:8.2} ms CPU, {:7.1} MB/s, {:7.1}M instructions, {:6.1}M cycles, IPC {:.2}; {:.0} instructions and {:.0} cycles a node", t.as_secs_f64() * 1e3, mb / t.as_secs_f64(), i as f64 / 1e6, cy as f64 / 1e6, i as f64 / cy as f64, i as f64 / nodes as f64, cy as f64 / nodes as f64);
    };
    for (name, b) in names.iter().zip(&best) {
        row(name, *b);
    }
    // Tier 1 by pass, each build's: each pass's least over the rounds, the record pass last.
    // A pass alone over the corpus finds each file out of the cache: shares, not a sum.
    let passes = jit.parser.tier1.pass_count();
    let mut by_pass = vec![[(std::time::Duration::MAX, u64::MAX, u64::MAX); 2]; passes];
    for _ in 0..rounds {
        for (w, (p, bs)) in [(&jit.parser, &mut bj), (&aot.parser, &mut ba)].into_iter().enumerate() {
            for k in 0..passes {
                let at = (thread_cpu(), retired());
                for b in bs.iter_mut() {
                    b.call_pass(&p.tier1, k);
                }
                let now = (thread_cpu(), retired());
                let (t, i, c) = (now.0 - at.0, now.1.0 - at.1.0, now.1.1 - at.1.1);
                let e = &mut by_pass[k][w];
                *e = (e.0.min(t), e.1.min(i), e.2.min(c));
            }
        }
    }
    let shapes = &jit.parser.tier1.passes;
    for (k, [j, a]) in by_pass.iter().enumerate() {
        let what = match shapes.get(k) {
            Some(s) if s.back => "back".to_string(),
            Some(s) => format!("forward, {} extracts, {} outlined", s.extracts, s.outlined.len()),
            None => "records".to_string(),
        };
        eprintln!("  tier 1 pass {k} ({what}): JIT {:6.1}M instructions {:6.1}M cycles; AOT {:6.1}M {:6.1}M", j.1 as f64 / 1e6, j.2 as f64 / 1e6, a.1 as f64 / 1e6, a.2 as f64 / 1e6);
    }
    let sum = |a: usize, b: usize| (best[a].0 + best[b].0, best[a].1 + best[b].1, best[a].2 + best[b].2);
    row("tier 1 + 2 (JIT)", sum(0, 2));
    row("tier 1 + 2 (AOT)", sum(1, 3));
}

/// Where tier 2's work goes: each opcode's runs a token over the corpus (the interpreter's
/// trace, `stream::run_seen`), and the rule calls.
#[test]
#[ignore = "a measurement: run by hand"]
fn tier2_words_a_token() {
    use rerail_compiler::program::split;
    let c = rerail_rr_parser::compiled().expect("compiles");
    let program = &c.program;
    let mut runs = vec![0u64; program.words.len()];
    let mut tokens = 0u64;
    for src in corpus() {
        let run = program.block_programs[0].run(src.as_bytes());
        tokens += run.codes.len() as u64;
        let _ = rerail_compiler::stream::run_seen(program, src.as_bytes(), &run, &mut |pc| runs[pc] += 1);
    }
    let mut by: std::collections::BTreeMap<String, u64> = Default::default();
    for (pc, &n) in runs.iter().enumerate() {
        let (op, mode, _) = split(program.words[pc]);
        *by.entry(format!("{op:#04x} m={}", if op == rerail_compiler::program::MARK { 0 } else { mode })).or_default() += n;
    }
    let mut v: Vec<(String, u64)> = by.into_iter().collect();
    v.sort_by_key(|x| std::cmp::Reverse(x.1));
    eprintln!("{tokens} tokens; words a token, by opcode:");
    for (k, n) in v {
        eprintln!("  {k}: {:.3}", n as f64 / tokens as f64);
    }
}

