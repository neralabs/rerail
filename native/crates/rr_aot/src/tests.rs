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

/// The word hash's multiplier is a literal where the hash is, as the lean lexer writes its
/// own (`word_slot`): no barrier hides it from rustc. And a word past eight bytes is compared
/// in the loop, straight-line, as the lean lexer's ninth byte: rr's words are 16 bytes at most
/// (`RecordTables::second`), so no tail compare is a function of its own.
#[test]
fn the_word_multiplier_is_a_literal() {
    let c = rerail_rr_parser::compiled().expect("compiles");
    let t = c.parser.tier1.records.as_ref().expect("the record pass");
    let src = include_str!("tier1_aot.rs");
    assert!(!src.contains("kept("), "a barrier on a constant");
    assert!(src.contains(&format!("{}i64 as u64", t.table.mult as i64)), "the multiplier a literal");
    assert!(t.second, "rr's words fit in two words each");
}

/// The token loop's innermost loop, the fused rules' (a word's hash in it), calls nothing: as
/// the lean lexer's `while run != 0`, it runs over the starts before the block's first rare one
/// (its strings', comments', numbers'), whose rules' calls are in the loop around it (`mid`).
/// Nor does it dispatch on the rules (a `match`): a rare start has its own head, as the lean
/// lexer's `match` on a rare token's first byte is apart from its run loop.
#[test]
fn the_run_loop_calls_nothing() {
    let c = rerail_rr_parser::compiled().expect("compiles");
    let mult = c.parser.tier1.records.as_ref().expect("the record pass").table.mult as i64;
    let src = include_str!("tier1_aot.rs");
    let f0 = &src[src.find("unsafe fn f0(").expect("the file loop")..];
    // The values that are the multiplier, then each hash's multiply by one.
    let vars: Vec<&str> = f0.match_indices(&format!(" = {mult}i64 as u64;")).map(|(at, _)| &f0[f0[..at].rfind(|c: char| !c.is_ascii_alphanumeric()).expect("its name") + 1..at]).collect();
    let hashes: Vec<usize> = vars.iter().flat_map(|v| {
        let at: Vec<usize> = f0.match_indices(&format!(".wrapping_mul({v})")).map(|(at, _)| at).collect();
        at
    }).collect();
    assert!(!hashes.is_empty(), "no word hash");
    // Each hash's innermost loop: a label before it whose last `continue` is after it.
    let calls: Vec<usize> = hashes.iter().map(|&h| {
        let mut inner: Option<(usize, usize)> = None;
        for (at, _) in f0.match_indices(": loop {") {
            let label = &f0[f0[..at].rfind('\'').expect("a label")..at];
            let Some(end) = f0.rfind(&format!("continue {label};")) else { continue };
            if at < h && h < end && inner.is_none_or(|(a, _)| at > a) {
                inner = Some((at, end));
            }
        }
        let (a, b) = inner.expect("a loop around the hash");
        f0[a..b].matches("(pin, ").count() + f0[a..b].matches("match ").count()
    }).collect();
    assert!(calls.contains(&0), "every word hash's loop has calls or a rules' match: {calls:?}");
}

/// A bitmask's reduction (`native_cpu_block::bitmask`, a vector's pairwise adds with itself)
/// is the 64-bit `addp` of its halves (`clif_rust`): rustc keeps one `addp` a step, simdjson's
/// five instructions, where it made each `vpaddq_u8(a, a)` three.
#[test]
fn a_bitmask_is_pairwise_adds() {
    let src = include_str!("tier1_aot.rs");
    let same = src.match_indices("vpaddq_u8(").filter(|(i, _)| {
        let rest = &src[i + "vpaddq_u8(".len()..];
        let args = &rest[..rest.find(')').unwrap_or(0)];
        args.split_once(", ").is_some_and(|(a, b)| a == b)
    });
    assert_eq!(same.count(), 0, "a vector's pairwise add with itself");
    assert!(src.contains("vcombine_u8(d, d)"), "no bitmask reduced by halves");
}

/// A call whose results are returned as they are is the return's (`clif_rust::returned_call`):
/// `return f(…)`, so rustc hands its caller's result on, a tail call, where a copy through a
/// temporary kept a short function's frame.
#[test]
fn a_returned_call_is_the_return() {
    let src = include_str!("tier1_aot.rs");
    assert!(src.contains("return unsafe { f"), "no call returned as it is");
    assert!(!src.contains("v0, v1, v2, v3, v4, v5) }; return ("), "a call's results copied into a return");
}

/// A short token's function (`native_cpu_block::RuleMode::Short`): a rule's function whose
/// calls are all its whole function's, of its own signature (a number's run past 16 bytes, a
/// comment or a string past its first 64 bytes, a string whose bytes its counts need), so its
/// way for a short token calls nothing, as a lexer reads a few digits, a line comment or a
/// plain string. A number's, a comment's and a string's at least.
#[test]
fn a_short_tokens_function_calls_only_its_whole() {
    use rerail_compiler::native_cpu_stream;
    native_cpu_stream::KEEP_IR.store(true, std::sync::atomic::Ordering::Relaxed);
    let c = rerail_rr_parser::compile_with(rerail_compiler::bitlex_masks::Shape::Rules).expect("compiles");
    let ir = &c.parser.tier1.ir;
    let calls = rerail_compiler::native_cpu_block::calls_of;
    let short = ir.iter().filter(|(_, f)| f.signature.params.len() == 6 && matches!(f.signature.returns.len(), 2 | 4)).filter(|(_, f)| {
        let made = calls(f);
        made.first().is_some_and(|&one| made.iter().all(|&x| x == one) && ir.iter().any(|(id, g)| *id == one && g.signature == f.signature))
    });
    assert!(short.count() >= 3, "a number's, a comment's and a string's short functions");
}

/// A rule's function returns its token in two words (`native_cpu_block::pack_rule`): two
/// registers, as a lexer's function returns its token, not four words through memory.
#[test]
fn a_rules_function_returns_two_words() {
    let src = include_str!("tier1_aot.rs");
    let six = "a0: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64)";
    assert!(src.contains(&format!("{six} -> (u64, u64) {{")), "no rule function of two words");
    assert!(!src.contains(&format!("{six} -> (u64, u64, u64, u64) {{")), "a rule function of four words");
}

/// Each variant over the corpus files the grammar parses, the least of `RR_ROUNDS` rounds: CPU
/// time, instructions, cycles, and MB a second of CPU. Tier 1 and tier 2 as the JIT built them
/// and as rustc did, each alone on batches and tapes made before; the whole parse a file at a
/// time (`image`: the batch, both tiers, the tape's bytes); the engine's parser
/// (`rr::engine_tape`); the lean parser's lexer and whole.
#[test]
#[ignore = "a benchmark: run by name, on an optimized build"]
fn jit_against_aot() {
    let jit = rerail_rr_parser::compiled().expect("compiles");
    let aot = crate::aot(BOTH).expect("the ahead-of-time build");
    let rounds: usize = std::env::var("RR_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(7);
    let files = corpus();
    let ours: Vec<&String> = files.iter().filter(|f| jit.parser.image(f.as_bytes()).is_ok()).collect();
    let nodes: usize = ours.iter().map(|f| rr::engine_tape(f).len()).sum();
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
    let names = ["tier 1 (JIT)", "tier 1 (AOT)", "tier 2 (JIT)", "tier 2 (AOT)", "whole (JIT)", "whole (AOT)", "Rust (rr::engine_tape)", "lean: lexer", "lean: whole", "lean: the digest"];
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
            n += rr::engine_tape(f).len();
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


/// A profile's load (`xctrace`): one form over the corpus for `RR_SECONDS` of CPU, `RR_FORM`
/// `tier1` (the ahead-of-time tier 1, its generated Rust with line tables) or `lexer` (the
/// lean lexer), to place the samples on their lines.
#[test]
#[ignore = "a profile's load: run by hand"]
fn aot_profile_loop() {
    let form = std::env::var("RR_FORM").unwrap_or_else(|_| "tier1".into());
    let seconds: f64 = std::env::var("RR_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(10.0);
    let files = corpus();
    // `jit1`: the JIT's tier 1 over every file, no ahead-of-time build (a scratch compile is
    // another program).
    let jit1 = form == "jit1";
    let aot = (!jit1).then(|| crate::aot(BOTH).expect("the ahead-of-time build"));
    // Every file: tier 1 runs on each, and a scratch compile may refuse others.
    let ours: Vec<&String> = files.iter().collect();
    let mut batches: Vec<_> = aot.iter().flat_map(|a| ours.iter().map(|f| rerail_compiler::native_cpu_block::Batch::new(&a.parser.tier1, &[f.as_bytes()]))).collect();
    let jit = rerail_rr_parser::compiled().expect("compiles");
    let mut jit_batches: Vec<_> = ours.iter().map(|f| rerail_compiler::native_cpu_block::Batch::new(&jit.parser.tier1, &[f.as_bytes()])).collect();
    let retired = rerail_compiler::native_cpu_codegen::retired;
    let (t0, (i0, c0)) = (thread_cpu(), retired());
    let (mut rounds, mut n) = (0usize, 0usize);
    while (thread_cpu() - t0).as_secs_f64() < seconds {
        match form.as_str() {
            "tier1" => batches.iter_mut().for_each(|b| b.call(&aot.as_ref().expect("built").parser.tier1)),
            "jit1" => jit_batches.iter_mut().for_each(|b| b.call(&jit.parser.tier1)),
            "lexer" => ours.iter().for_each(|f| n += rr::lean::token_count(f)),
            other => panic!("RR_FORM={other}: tier1 or lexer"),
        }
        rounds += 1;
    }
    std::hint::black_box(n);
    let (t1, (i1, c1)) = (thread_cpu(), retired());
    eprintln!("{form}: {rounds} rounds, {:.1}M instructions and {:.1}M cycles a round, {:?} CPU; {} a round", (i1 - i0) as f64 / rounds as f64 / 1e6, (c1 - c0) as f64 / rounds as f64 / 1e6, (t1 - t0) / rounds as u32, n / rounds.max(1));
}

/// Tier 1's instructions a token against the lean lexer's, a kind of token at a time: a file
/// of words, of punctuation, of strings, of numbers, of comments, each about 64 KB. Where the
/// corpus's average hides which tokens cost what, the side-by-side count by kind.
#[test]
#[ignore = "a measurement: run by hand"]
fn tier1_by_kind() {
    let aot = crate::aot(BOTH).expect("the ahead-of-time build");
    let retired = rerail_compiler::native_cpu_codegen::retired;
    let rounds = 21;
    let kinds: [(&str, &str); 6] = [
        ("words", "alpha beta_2 gamma delta epsilon zeta eta theta iota kappa\n"),
        ("punctuation", "(),:-;[]{}.=<>!=<=>=:-\n"),
        ("mixed code", "rule(X, Y) :- edge(X, Z), path(Z, Y), X != Y.\n"),
        ("strings", "\"select * from t\" \"a\" \"some longer text, with spaces in it\"\n"),
        ("numbers", "1 22 333 4444 55555 1.5 2.25\n"),
        ("comments", "// a line comment of some words\n/* a block */\n"),
    ];
    // `RR_LINE`: that line alone (a kind's cost by its tokens' lengths).
    let line = std::env::var("RR_LINE").ok();
    let kinds: Vec<(&str, &str)> = match &line {
        Some(l) => vec![("line", l.as_str())],
        None => kinds.to_vec(),
    };
    // `RR_KIND`: that kind alone, tier 1 for `RR_SECONDS`, for a profile.
    let only = std::env::var("RR_KIND").ok();
    let seconds: f64 = std::env::var("RR_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    for (name, line) in kinds {
        if only.as_deref().is_some_and(|k| k != name) {
            continue;
        }
        let text = line.repeat(65536 / line.len());
        // Lean's count has no comment (it keeps them apart, read all the same): each comment
        // counted too, for both.
        let comments = text.matches("//").count() + text.matches("/*").count();
        let tokens = rr::lean::token_count(&text) + comments;
        if only.is_some() {
            let mut b = rerail_compiler::native_cpu_block::Batch::new(&aot.parser.tier1, &[text.as_bytes()]);
            let t0 = thread_cpu();
            while (thread_cpu() - t0).as_secs_f64() < seconds {
                b.call(&aot.parser.tier1);
            }
            continue;
        }
        let mut b = rerail_compiler::native_cpu_block::Batch::new(&aot.parser.tier1, &[text.as_bytes()]);
        b.call(&aot.parser.tier1);
        let (mut ours, mut lean) = (u64::MAX, u64::MAX);
        for _ in 0..rounds {
            let at = retired().0;
            b.call(&aot.parser.tier1);
            ours = ours.min(retired().0 - at);
            let at = retired().0;
            std::hint::black_box(rr::lean::token_count(std::hint::black_box(&text)));
            lean = lean.min(retired().0 - at);
        }
        eprintln!("  {name:>12}: {tokens:6} tokens; tier 1 AOT {:6.1} instructions a token, lean lexer {:6.1}", ours as f64 / tokens as f64, lean as f64 / tokens as f64);
    }
}

/// The loop's arm of a first byte `b` in the generated Rust: from `b => {` in the rules'
/// `match` to the next arm's pattern.
fn arm_of<'s>(f0: &'s str, b: &str) -> &'s str {
    let m = f0.find(" { 34 => {").expect("the rules' match");
    let body = &f0[m..];
    let at = body.find(&format!("{b} => {{")).expect("the arm");
    let rest = &body[at + b.len() + 5..];
    let end = [" 35 => {", " 47 => {", " 48..=57 => {", " 96 => {", " _ => {"].iter().filter_map(|p| rest.find(p)).min().expect("the next arm");
    &rest[..end]
}

/// The string's arm reads like lean's `string_end` from the byte after the quote, its carry
/// none: no byte before its quote read, nothing called before its window
/// (`native_cpu_block::before_start`: no token ends with a `\` before a `"` the loop starts).
#[test]
fn a_strings_arm_reads_nothing_before_its_quote() {
    let src = include_str!("tier1_aot.rs");
    let f0 = &src[src.find("unsafe fn f0(").expect("the file loop")..];
    let arm = arm_of(f0, "34");
    let window = arm.find("vld1q_u8").expect("the string's first window");
    assert!(!arm[..window].contains("(pin, "), "a call before the string's window: {}", &arm[..window]);
    assert!(!arm[..window].contains("-1i64 as u64);\nv"), "a read before the quote: {}", &arm[..window]);
}

/// The `/` arm reads like lean's: `//` and `/*` scanned in place, their long reads the whole
/// reader's one call; `/` alone its pair test inline (no rule function), its code the
/// punctuation's (no word hash), its next start the token's end (no `max`).
#[test]
fn a_slashs_arm_is_leans() {
    let c = rerail_rr_parser::compiled().expect("compiles");
    let mult = c.parser.tier1.records.as_ref().expect("the record pass").table.mult as i64;
    let src = include_str!("tier1_aot.rs");
    let f0 = &src[src.find("unsafe fn f0(").expect("the file loop")..];
    let arm = arm_of(f0, "47");
    let mut called: Vec<&str> = arm.match_indices("(pin, ").map(|(at, _)| &arm[arm[..at].rfind(' ').expect("a callee") + 1..at]).collect();
    called.sort();
    called.dedup();
    assert_eq!(called.len(), 1, "the arm calls the whole reader alone: {called:?}");
    assert!(!arm.contains(&format!("{mult}i64")), "a word hash in the `/` arm");
    assert!(!arm.contains("as i64).max("), "a `max` in the `/` arm");
}
