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

/// Every corpus file: the ahead-of-time build writes the JIT's tape, and refuses where it does.
#[test]
#[ignore = "the corpus: run by name"]
fn aot_tapes_are_the_jits() {
    let jit = rerail_rr_parser::compiled().expect("compiles");
    let aot = crate::aot().expect("the ahead-of-time build");
    let files = corpus();
    let mut differ = Vec::new();
    let mut taken = 0;
    for (i, f) in files.iter().enumerate() {
        let (a, j) = (aot.parser.columns(f.as_bytes()), jit.parser.columns(f.as_bytes()));
        match (a, j) {
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
    assert_eq!(differ, Vec::<usize>::new(), "files whose tape is not the JIT's");
}

/// Each variant over the corpus files the grammar parses: tier 1, tier 2 as the JIT built it
/// and as rustc did, the engine's parser, and the lean parser (lexer, whole). The least of
/// `RR_ROUNDS` rounds, CPU time, instructions and cycles.
#[test]
#[ignore = "a benchmark: run by name, on an optimized build"]
fn jit_against_aot() {
    let jit = rerail_rr_parser::compiled().expect("compiles");
    let aot = crate::aot().expect("the ahead-of-time build");
    let rounds: usize = std::env::var("RR_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(7);
    let files = corpus();
    let ours: Vec<&String> = files.iter().filter(|f| jit.parser.columns(f.as_bytes()).is_ok()).collect();
    let nodes: usize = ours.iter().map(|f| rr::tape(f).len()).sum();
    let bytes: usize = ours.iter().map(|f| f.len()).sum();
    let p = &jit.parser;
    let mut batches: Vec<_> = ours.iter().map(|f| rerail_compiler::native_cpu_block::Batch::new(&p.tier1, &[f.as_bytes()])).collect();
    for b in &mut batches {
        b.call(&p.tier1);
    }
    let mut tapes: Vec<_> = batches.iter().map(|b| p.tape(b)).collect();
    let retired = rerail_compiler::native_cpu_codegen::retired;
    let names = ["tier 1", "tier 2 (JIT)", "tier 2 (AOT)", "Rust (rr::tape)", "lean: lexer", "lean: whole"];
    let mut best = [(std::time::Duration::MAX, u64::MAX, u64::MAX); 6];
    for _ in 0..rounds {
        let mut at = (thread_cpu(), retired());
        let mut lap = |k: usize, best: &mut [(std::time::Duration, u64, u64); 6]| {
            let now = (thread_cpu(), retired());
            let (t, i, c) = (now.0 - at.0, now.1.0 - at.1.0, now.1.1 - at.1.1);
            best[k] = (best[k].0.min(t), best[k].1.min(i), best[k].2.min(c));
            at = (thread_cpu(), retired());
        };
        for b in &mut batches {
            b.call(&p.tier1);
        }
        lap(0, &mut best);
        for (b, t) in batches.iter().zip(&mut tapes) {
            let _ = jit.parser.words(b, t);
        }
        lap(1, &mut best);
        for (b, t) in batches.iter().zip(&mut tapes) {
            let _ = aot.parser.words(b, t);
        }
        lap(2, &mut best);
        let mut n = 0;
        for f in &ours {
            n += rr::tape(f).len();
        }
        std::hint::black_box(n);
        lap(3, &mut best);
        for f in &ours {
            n += rr::lean::token_count(f);
        }
        std::hint::black_box(n);
        lap(4, &mut best);
        for f in &ours {
            n += rr::lean::tape(f).len();
        }
        std::hint::black_box(n);
        lap(5, &mut best);
    }
    eprintln!("{} files the grammar parses, {:.2} MB, {nodes} nodes; the least of {rounds} rounds", ours.len(), bytes as f64 / 1e6);
    for (name, (t, i, cy)) in names.iter().zip(best) {
        eprintln!("  {name:>16}: {t:8.2?} CPU, {:7.1}M instructions, {:7.1}M cycles, IPC {:.2}; {:.0} instructions and {:.0} cycles a node", i as f64 / 1e6, cy as f64 / 1e6, i as f64 / cy as f64, i as f64 / nodes as f64, cy as f64 / nodes as f64);
    }
}
