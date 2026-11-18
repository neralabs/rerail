//! The ahead-of-time build against the JIT's and the lean parser: the same tapes, then each
//! one's cost, in instructions, cycles and the thread's CPU time.

use rerail_kotlin_lean as lean;

/// The corpus: `RERAIL_KOTLIN_CORPUS`'s `.kt` files, as `rerail-kotlin-parser`'s corpus test
/// reads them.
fn corpus() -> Vec<String> {
    let dir = std::env::var("RERAIL_KOTLIN_CORPUS").expect("set RERAIL_KOTLIN_CORPUS, a folder of .kt files");
    let mut paths: Vec<_> = std::fs::read_dir(dir).expect("the corpus").flatten().map(|e| e.path()).collect();
    paths.sort();
    paths.iter().filter_map(|p| std::fs::read_to_string(p).ok()).collect()
}

/// This thread's CPU time.
fn thread_cpu() -> std::time::Duration {
    let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: clock_gettime writes the structure.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    std::time::Duration::new(t.tv_sec as u64, t.tv_nsec as u32)
}

const BOTH: crate::Tiers = crate::Tiers { one: true, two: true };

/// The ahead-of-time build, its proofs answered by the JIT's compile (decision 0402).
fn both() -> (&'static rerail_kotlin_parser::Compiled, rerail_kotlin_parser::Compiled) {
    let jit = rerail_kotlin_parser::compiled().expect("compiles");
    let aot = crate::aot(BOTH, jit.proofs.iter().cloned().collect()).expect("the ahead-of-time build");
    (jit, aot)
}

/// Every corpus file: the ahead-of-time tier 1 writes the JIT's masks, positions and counts;
/// both tiers ahead of time write the JIT's tape, and refuse where it does.
#[test]
#[ignore = "reads RERAIL_KOTLIN_CORPUS: run by name"]
fn aot_tapes_are_the_jits() {
    let (jit, aot) = both();
    let files = corpus();
    let (mut differ, mut tier1_differ, mut taken) = (Vec::new(), Vec::new(), 0);
    for (i, f) in files.iter().enumerate() {
        let mut a = rerail_compiler::native_cpu_block::Batch::new(&aot.parser.tier1, &[f.as_bytes()]);
        let mut j = rerail_compiler::native_cpu_block::Batch::new(&jit.parser.tier1, &[f.as_bytes()]);
        a.call(&aot.parser.tier1);
        j.call(&jit.parser.tier1);
        if a.out != j.out || a.positions != j.positions || a.counts != j.counts {
            tier1_differ.push(i);
        }
        match (aot.parser.image(f.as_bytes()), jit.parser.image(f.as_bytes())) {
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

/// The JIT's code against rustc's, and both against the lean parser, over the corpus files the
/// grammar parses: each part's least over the rounds (`KOTLIN_ROUNDS`, 7), in the thread's CPU
/// time, instructions and cycles.
#[test]
#[ignore = "a benchmark: run by name, on an optimized build"]
fn jit_against_aot() {
    let (jit, aot) = both();
    let rounds: usize = std::env::var("KOTLIN_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(7);
    let files = corpus();
    let ours: Vec<&String> = files.iter().filter(|f| jit.parser.image(f.as_bytes()).is_ok()).collect();
    let nodes: usize = ours.iter().map(|f| lean::tape(f.as_bytes()).map_or(0, |t| t.code.len() / 2)).sum();
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
    let names = ["tier 1 (JIT)", "tier 1 (AOT)", "tier 2 (JIT)", "tier 2 (AOT)", "whole (JIT)", "whole (AOT)", "lean: lexer", "lean: whole"];
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
            n += jit.parser.image(f.as_bytes()).map_or(0, |c| c.len());
        }
        lap(4, &mut best);
        for f in &ours {
            n += aot.parser.image(f.as_bytes()).map_or(0, |c| c.len());
        }
        lap(5, &mut best);
        for f in &ours {
            n += lean::lex_only(f.as_bytes());
        }
        lap(6, &mut best);
        for f in &ours {
            n += lean::tape(f.as_bytes()).map_or(0, |t| t.code.len());
        }
        std::hint::black_box(n);
        lap(7, &mut best);
    }
    let mb = bytes as f64 / 1e6;
    eprintln!("{} files the grammar parses (of {}), {mb:.2} MB, {nodes} nodes; the least of {rounds} rounds", ours.len(), files.len());
    let row = |name: &str, (t, i, cy): (std::time::Duration, u64, u64)| {
        eprintln!("  {name:>18}: {:8.2} ms CPU, {:7.1} MB/s, {:8.1}M instructions, {:7.1}M cycles, IPC {:.2}; {:.0} instructions and {:.0} cycles a node", t.as_secs_f64() * 1e3, mb / t.as_secs_f64(), i as f64 / 1e6, cy as f64 / 1e6, i as f64 / cy as f64, i as f64 / nodes as f64, cy as f64 / nodes as f64);
    };
    for (name, b) in names.iter().zip(&best) {
        row(name, *b);
    }
    let sum = |a: usize, b: usize| (best[a].0 + best[b].0, best[a].1 + best[b].1, best[a].2 + best[b].2);
    row("tier 1 + 2 (JIT)", sum(0, 2));
    row("tier 1 + 2 (AOT)", sum(1, 3));
}
