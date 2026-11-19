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

/// The ahead-of-time build, its proofs answered by the JIT's compile (decision 0404).
fn both() -> (&'static rerail_kotlin_parser::Compiled, rerail_kotlin_parser::Compiled) {
    let jit = rerail_kotlin_parser::compiled().expect("compiles");
    let aot = crate::aot(BOTH, jit.proofs.iter().cloned().collect()).expect("the ahead-of-time build");
    (jit, aot)
}

/// Every corpus file: the ahead-of-time tier 1 writes the JIT's records, counts, and the masks
/// tier 2 reads from its words (`BlockNative::masks`, but those tier 2 reads from the bytes:
/// `records_made`); both tiers ahead of time write the JIT's tape, and refuse where it does. The
/// JIT's body writes other masks too, which nothing reads; the generated tier 1 makes none of
/// them (decision 0432).
#[test]
#[ignore = "reads RERAIL_KOTLIN_CORPUS: run by name"]
fn aot_tapes_are_the_jits() {
    let (jit, aot) = both();
    // The corpus has no lost comment (`SETTLE m=2`): the texts that have one too.
    let files: Vec<String> = corpus().into_iter().chain(rerail_kotlin_parser::lost_comment_texts()).collect();
    let native = rerail_compiler::block_native::BlockNative::lower(&jit.program.block_programs[0]);
    let unmade = native.records_made();
    let read: Vec<usize> = native.masks.iter().copied().filter(|o| !unmade.contains(o)).collect();
    let n = jit.parser.tier1.outputs;
    let extracts = jit.parser.tier1.extracts.len();
    assert_eq!(jit.parser.tier1.records.as_ref().map(|t| t.layout), Some(rerail_compiler::records::RecordLayout::Rows), "the records a row a token, from the file's first position");
    let (mut differ, mut tier1_differ, mut taken) = (Vec::new(), Vec::new(), 0);
    for (i, f) in files.iter().enumerate() {
        let mut a = rerail_compiler::native_cpu_block::Batch::new(&aot.parser.tier1, &[f.as_bytes()]);
        let mut j = rerail_compiler::native_cpu_block::Batch::new(&jit.parser.tier1, &[f.as_bytes()]);
        a.call(&aot.parser.tier1);
        j.call(&jit.parser.tier1);
        let masks = (0..a.out.len()).all(|k| !read.contains(&(k % n)) || a.out[k] == j.out[k]);
        // A file tier 1 refuses (its flag, the count after the extracts'): its records up to the
        // count, as the JIT's loop stops at the refusal and nothing reads past it.
        let refused = j.counts.get(extracts).is_some_and(|&t| t != 0);
        let positions = if refused {
            let records = 3 * j.counts[0] as usize;
            a.positions[..records] == j.positions[..records]
        } else {
            a.positions == j.positions
        };
        if !masks || !positions || a.counts != j.counts {
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

/// The compiled parser's broad structure, as text: tier 1's passes and their shapes, the block
/// program's operations by kind, and tier 2's size. A tool for comparing with the lean parser's
/// (`block.rs`'s stage 1, `lex.rs`'s token loop, `parse/` a function a rule).
#[test]
#[ignore = "a tool: run by name"]
fn structure() {
    let jit = rerail_kotlin_parser::compiled().expect("compiles");
    let b = &jit.program.block_programs[0];
    let mut kinds = std::collections::BTreeMap::<String, usize>::new();
    for op in &b.ops {
        let name = format!("{op:?}");
        let name = name.split(['(', ' ', '{']).next().unwrap_or("").to_string();
        *kinds.entry(name).or_default() += 1;
    }
    eprintln!("tier 1: {} operations, {} fixed points, {} outputs", b.ops.len(), b.rounds.len(), b.outputs.len());
    eprintln!("  by kind: {kinds:?}");
    for (k, p) in jit.parser.tier1.passes.iter().enumerate() {
        eprintln!(
            "  pass {k}: back {}, block loop {}, general {}, extracts {}, outlined {}, rare branches {}, gate branches {}, round branches {}, region calls {}, reads back {}",
            p.back, p.block_loop, p.general, p.extracts, p.outlined.len(), p.rare_branches, p.gate_branches, p.round_branches, p.region_calls.len(), p.reads_back
        );
    }
    eprintln!("tier 1 passes in all: {}", jit.parser.tier1.pass_count());
    let native = rerail_compiler::block_native::BlockNative::lower(b);
    let passes = native.passes();
    for (k, p) in passes.passes.iter().enumerate() {
        match p {
            rerail_compiler::block_native::Pass::Forward { body, stores } => eprintln!("  native pass {k}: forward, {} instructions, {} stored for later passes", body.insts.len(), stores.len()),
            rerail_compiler::block_native::Pass::Back(fills) => eprintln!("  native pass {k}: back, {} back_fills", fills.len()),
        }
    }
    eprintln!("  slots a block: {}", passes.slots);
    // The CPU's form (`BlockNative::cpu_form`): the record pass's starts and lasts left out, the
    // regions walked; what the CPU's tier 1 runs.
    let cpu = native.cpu_form();
    let mut kinds = std::collections::BTreeMap::<String, usize>::new();
    for i in &cpu.insts {
        let name = format!("{i:?}");
        *kinds.entry(name.split(['(', ' ', '{']).next().unwrap_or("").to_string()).or_default() += 1;
    }
    eprintln!("  CPU form: {} instructions, {} outputs; by kind: {kinds:?}", cpu.insts.len(), cpu.outputs.len());
    eprintln!("tier 2: {} words, {} lines of machine", jit.words.lines().count(), jit.machine.lines().count());
    // The records' marks (`Records::marks`): each its output and word, and how many of the
    // corpus's bytes its output holds at (where `RERAIL_KOTLIN_CORPUS` is set).
    if let Some(r) = &b.records {
        let files: Vec<String> = std::env::var("RERAIL_KOTLIN_CORPUS").map(|_| corpus()).unwrap_or_default();
        let mut bits = vec![0u64; b.outputs.len()];
        for f in &files {
            let mut batch = rerail_compiler::native_cpu_block::Batch::new(&jit.parser.tier1, &[f.as_bytes()]);
            batch.call(&jit.parser.tier1);
            for (i, w) in batch.out.iter().enumerate() {
                bits[i % b.outputs.len()] += u64::from(w.count_ones());
            }
        }
        eprintln!("records' marks ({} files): gaps {:#x}, tested only {:#x}", files.len(), r.gaps, r.tested_only);
        for (k, &(o, w)) in r.marks.iter().enumerate() {
            let held = bits.get(usize::from(o)).copied().unwrap_or(0);
            eprintln!("  mark {k}: output {o}, word {w}, {held} bits");
        }
    }
}

/// The ahead-of-time tier 1 alone over the corpus files the grammar parses, `KOTLIN_ROUNDS`
/// times (50): for a profiler (`xctrace`'s Time Profiler) to say where its time goes.
#[test]
#[ignore = "a tool for a profiler: run by name, on an optimized build"]
fn tier1_alone() {
    let (jit, aot) = both();
    let rounds: usize = std::env::var("KOTLIN_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(50);
    let files = corpus();
    let ours: Vec<&String> = files.iter().filter(|f| jit.parser.image(f.as_bytes()).is_ok()).collect();
    let mut batches: Vec<_> = ours.iter().map(|f| rerail_compiler::native_cpu_block::Batch::new(&aot.parser.tier1, &[f.as_bytes()])).collect();
    let at = thread_cpu();
    for _ in 0..rounds {
        for b in &mut batches {
            b.call(&aot.parser.tier1);
        }
    }
    eprintln!("{} files, {rounds} rounds: {:.1} ms CPU a round", ours.len(), (thread_cpu() - at).as_secs_f64() * 1e3 / rounds as f64);
}
