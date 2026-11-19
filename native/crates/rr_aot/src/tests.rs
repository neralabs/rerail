//! The ahead-of-time build against the JIT's: the same tapes, then each one's cost.

use rerail_rr_syntax as rr;

/// The repository's `.rerail` files.
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
            } else if p.extension().is_some_and(|x| x == "rerail") {
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
/// in the loop by its ninth byte alone, as the lean lexer's `word_code`: rr's words are nine
/// bytes at most (`RecordTables::rest`), so no second key is read and no tail compare is a
/// function of its own.
#[test]
fn the_word_multiplier_is_a_literal() {
    let c = rerail_rr_parser::compiled().expect("compiles");
    let t = c.parser.tier1.records.as_ref().expect("the record pass");
    let src = include_str!(concat!(env!("OUT_DIR"), "/tier1_aot.rs"));
    assert!(!src.contains("kept("), "a barrier on a constant");
    assert!(src.contains(&format!("{}i64 as u64", t.table.mult as i64)), "the multiplier a literal");
    assert_eq!(t.rest, rerail_compiler::native_cpu_block::WordRest::Ninth, "rr's words are nine bytes at most");
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
    let src = include_str!(concat!(env!("OUT_DIR"), "/tier1_aot.rs"));
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

/// Each word lookup is lean's `word_code`: one 16-byte slot (key and meta), its key and length
/// compared, and the ninth byte where the token has nine (`(len != 9) | (ninth == ..)`), no
/// second key read from a 32-byte slot.
#[test]
fn a_words_lookup_is_leans() {
    let c = rerail_rr_parser::compiled().expect("compiles");
    let mult = c.parser.tier1.records.as_ref().expect("the record pass").table.mult as i64;
    let src = include_str!(concat!(env!("OUT_DIR"), "/tier1_aot.rs"));
    let f0 = &src[src.find("unsafe fn f0(").expect("the file loop")..];
    let vars: Vec<&str> = f0.match_indices(&format!(" = {mult}i64 as u64;")).map(|(at, _)| &f0[f0[..at].rfind(|c: char| !c.is_ascii_alphanumeric()).expect("its name") + 1..at]).collect();
    let hashes: Vec<usize> = vars.iter().flat_map(|v| f0.match_indices(&format!(".wrapping_mul({v})")).map(|(at, _)| at).collect::<Vec<_>>()).collect();
    assert!(!hashes.is_empty(), "no word hash");
    for h in hashes {
        let look = &f0[h..];
        let look = &look[..look.find(" = if ").expect("the code chosen on a hit")];
        assert!(look.contains(".wrapping_shl(4)"), "a slot of 16 bytes: {look}");
        assert!(!look.contains("wrapping_add(16isize"), "a second key read: {look}");
        assert!(look.contains("((9i64 as u64) as u64)"), "the ninth byte asked where the token has nine: {look}");
        // The slot's length and ninth byte loaded alone (lean's `ldrb`s), not cut from its
        // meta word an instruction after its load.
        assert!(look.contains("wrapping_add(8isize as usize)) as *const u8)") && !look.contains(".wrapping_shr(56)"), "the slot's length and ninth as bytes: {look}");
    }
    // The key cut to its length as lean's `u64::MAX >> (64 - 8 * len)`: a shift by 64 less
    // its bits, not by their negation (`(56 * n) & 56` to LLVM).
    let negs = f0.match_indices(".wrapping_neg();\n").filter(|&(at, _)| f0[at..].lines().nth(1).is_some_and(|l| l.ends_with("= -1i64 as u64;"))).count();
    assert_eq!(negs, 0, "a key cut by a negated shift");
    assert!(f0.contains("(64i64 as u64).wrapping_sub("), "a key cut by 64 less its bits");
}

/// A word's or a punctuation's end is lean's `pos + words.max(n1)`: the run from its first
/// byte (none at a punctuation's) and 1, or 2 at the pair word's bit; one branch before it, to
/// the run past the block or a byte of neither rule, not a branch on the entry's rule bits and
/// a `select` between a pair's last and a run's.
#[test]
fn a_tokens_end_is_leans_max() {
    let src = include_str!(concat!(env!("OUT_DIR"), "/tier1_aot.rs"));
    let f0 = &src[src.find("unsafe fn f0(").expect("the file loop")..];
    // The run loop's end: the first unsigned `max` of two values (`vN.max(vM)`; a signed one
    // is `(vN as i64).max(..)`).
    let at = f0.match_indices(".max(v").map(|(at, _)| at).find(|&at| f0[..at].ends_with(|c: char| c.is_ascii_digit())).expect("a max");
    let head = f0[..at].rfind(": loop {").expect("the run loop");
    let body = &f0[head..at];
    assert!(!body.contains("(pin, ") && !body.contains("loop {\n'"), "the run loop's head before its end: {body}");
    assert_eq!(body.matches("if ").count(), 1, "the branches before a token's end: {body}");
    assert!(body.contains(".trailing_zeros()"), "the run from the token's first byte: {body}");
}

/// A token's record is written once, at its end, as the lean lexer's `out.add(len).write(Tok
/// {..})`: in the run loop its start, last and code are three adjacent stores (which rustc
/// pairs), not the start at the token's head and the rest as they are made.
#[test]
fn a_tokens_record_is_written_once() {
    let src = include_str!(concat!(env!("OUT_DIR"), "/tier1_aot.rs"));
    let f0 = &src[src.find("unsafe fn f0(").expect("the file loop")..];
    // The run loop: the innermost loop around the token's end (`a_tokens_end_is_leans_max`), to
    // its `continue`.
    let at = f0.match_indices(".max(v").map(|(at, _)| at).find(|&at| f0[..at].ends_with(|c: char| c.is_ascii_digit())).expect("a max");
    let head = f0[..at].rfind(": loop {").expect("the run loop");
    let label = &f0[f0[..head].rfind('\'').expect("its label")..head];
    let body = &f0[head..head + f0[head..].find(&format!("continue {label};")).expect("its continue")];
    let lines: Vec<&str> = body.lines().collect();
    let writes: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| l.contains("write_unaligned") && l.contains("*mut u32")).map(|(i, _)| i).collect();
    assert_eq!(writes.len(), 3, "the record's stores in the run loop: {body}");
    assert!(writes.windows(2).all(|w| w[1] == w[0] + 1), "the record's stores apart: {body}");
}

/// The refill classifies a block's bytes as the lean lexer's window does (`micro`): one
/// nibble lookup a 16 bytes for the space, word and rare sets (`class_pack`, their products in
/// one table), and the pair's next bytes an `ext` of two loaded vectors, no load at `+1`.
#[test]
fn the_refill_is_leans_window() {
    let src = include_str!(concat!(env!("OUT_DIR"), "/tier1_aot.rs"));
    let f0 = &src[src.find("unsafe fn f0(").expect("the file loop")..];
    let from = f0.find("vld1q_u8(").expect("the refill's loads");
    let to = from + f0[from..].find("vextq_u8::<1>(").expect("the pair's next bytes by `ext`");
    let window = &f0[from..to];
    let mut tables: Vec<&str> = window.match_indices("vqtbl1q_u8(").map(|(i, m)| {
        let rest = &window[i + m.len()..];
        &rest[..rest.find(',').unwrap_or(0)]
    }).collect();
    tables.sort_unstable();
    tables.dedup();
    assert_eq!(tables.len(), 2, "the classes' lookups, one table's halves: {tables:?}\n{window}");
    let odd = f0.lines().filter(|l| l.contains("vld1q_u8(") && [1, 17, 33, 49].iter().any(|off| l.contains(&format!("wrapping_add({off}isize as usize)"))));
    assert_eq!(odd.count(), 0, "a vector load at a block's byte after");
}

/// A token's test of the byte after its first (`-` not before a digit) reads it as lean reads
/// `b[pos + 1]`, the file's padding a zero: no bound on the token's own byte (`within` at the
/// anchor, `native_cpu_bytes::scalar`), no signed compare with the file's length.
#[test]
fn a_tests_next_byte_asks_no_bound() {
    let src = include_str!(concat!(env!("OUT_DIR"), "/tier1_aot.rs"));
    let f0 = &src[src.find("unsafe fn f0(").expect("the file loop")..];
    let at = f0.find("== ((45i64 as u64) as u64)").expect("the `-` test");
    let test = &f0[at..at + f0[at..].find("wrapping_shl(").expect("its hit placed")];
    assert!(!test.contains("as i64)"), "a bound in the `-` test: {test}");
}

/// A cold block stays a branch (`clif_rust::COLD`): each `cold_path` with the empty `asm!` that
/// keeps LLVM from folding the arm into a `select`. Folded, the refill's next block waited on
/// the block's last token's end, the refill's loads after the token chain.
#[test]
fn a_cold_block_stays_a_branch() {
    let src = include_str!(concat!(env!("OUT_DIR"), "/tier1_aot.rs"));
    let cold = src.matches("core::hint::cold_path();").count();
    assert!(cold > 0, "no cold block");
    assert_eq!(src.matches("core::hint::cold_path(); unsafe { core::arch::asm!(\"\", options(nomem, nostack, preserves_flags)) };").count(), cold, "a cold block rustc may fold");
}

/// A bitmask's reduction (`native_cpu_block::bitmask`, a vector's pairwise adds with itself)
/// is the 64-bit `addp` of its halves (`clif_rust`): rustc keeps one `addp` a step, simdjson's
/// five instructions, where it made each `vpaddq_u8(a, a)` three.
#[test]
fn a_bitmask_is_pairwise_adds() {
    let src = include_str!(concat!(env!("OUT_DIR"), "/tier1_aot.rs"));
    let same = src.match_indices("vpaddq_u8(").filter(|(i, _)| {
        let rest = &src[i + "vpaddq_u8(".len()..];
        let args = &rest[..rest.find(')').unwrap_or(0)];
        args.split_once(", ").is_some_and(|(a, b)| a == b)
    });
    assert_eq!(same.count(), 0, "a vector's pairwise add with itself");
    assert!(src.contains("vcombine_u8(d, d)"), "no bitmask reduced by halves");
}

/// A short token's form (`native_cpu_block::RuleMode::Short`) is the loop's arm: the number's,
/// the string's and the `/` arm call nothing but their rule's whole function (a number's run
/// past 16 bytes, a comment or a string past its first window), as the lean lexer reads a few
/// digits, a line comment or a plain string in its `match` arm. (The returned call these short
/// functions made is `clif_rust`'s `a_returned_call_is_the_return`.)
#[test]
fn a_short_tokens_arm_calls_only_its_whole() {
    let src = include_str!(concat!(env!("OUT_DIR"), "/tier1_aot.rs"));
    let f0 = &src[src.find("unsafe fn f0(").expect("the file loop")..];
    for b in ["34", "47"] {
        let arm = arm_of(f0, b);
        let mut called: Vec<&str> = arm.match_indices("(pin, ").map(|(at, _)| &arm[arm[..at].rfind(' ').expect("a callee") + 1..at]).collect();
        called.sort();
        called.dedup();
        assert!(called.len() <= 1, "the {b} arm calls more than its whole function: {called:?}");
    }
}

/// A rule's function returns its token in two words (`native_cpu_block::pack_rule`): two
/// registers, as a lexer's function returns its token, not four words through memory.
#[test]
fn a_rules_function_returns_two_words() {
    let src = include_str!(concat!(env!("OUT_DIR"), "/tier1_aot.rs"));
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
    // `RR_KIND`: that kind alone, tier 1 (or lean, `RR_WHO`) for `RR_SECONDS`, for a profile.
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
            // `RR_WHO=lean`: the lean lexer alone, for the same profile of it.
            let lean = std::env::var("RR_WHO").is_ok_and(|w| w == "lean");
            let mut b = rerail_compiler::native_cpu_block::Batch::new(&aot.parser.tier1, &[text.as_bytes()]);
            let t0 = thread_cpu();
            while (thread_cpu() - t0).as_secs_f64() < seconds {
                if lean {
                    std::hint::black_box(rr::lean::token_count(std::hint::black_box(&text)));
                } else {
                    b.call(&aot.parser.tier1);
                }
            }
            continue;
        }
        let mut b = rerail_compiler::native_cpu_block::Batch::new(&aot.parser.tier1, &[text.as_bytes()]);
        b.call(&aot.parser.tier1);
        let (mut ours, mut lean) = ((u64::MAX, u64::MAX), (u64::MAX, u64::MAX));
        let least = |m: (u64, u64), a: (u64, u64), b: (u64, u64)| (m.0.min(b.0 - a.0), m.1.min(b.1 - a.1));
        for _ in 0..rounds {
            let at = retired();
            b.call(&aot.parser.tier1);
            ours = least(ours, at, retired());
            let at = retired();
            std::hint::black_box(rr::lean::token_count(std::hint::black_box(&text)));
            lean = least(lean, at, retired());
        }
        let t = tokens as f64;
        eprintln!(
            "  {name:>12}: {tokens:6} tokens; tier 1 AOT {:6.1} instructions {:5.1} cycles a token (IPC {:.2}), lean lexer {:6.1} {:5.1} (IPC {:.2})",
            ours.0 as f64 / t, ours.1 as f64 / t, ours.0 as f64 / ours.1 as f64, lean.0 as f64 / t, lean.1 as f64 / t, lean.0 as f64 / lean.1 as f64
        );
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
    let src = include_str!(concat!(env!("OUT_DIR"), "/tier1_aot.rs"));
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
    let src = include_str!(concat!(env!("OUT_DIR"), "/tier1_aot.rs"));
    let f0 = &src[src.find("unsafe fn f0(").expect("the file loop")..];
    let arm = arm_of(f0, "47");
    let mut called: Vec<&str> = arm.match_indices("(pin, ").map(|(at, _)| &arm[arm[..at].rfind(' ').expect("a callee") + 1..at]).collect();
    called.sort();
    called.dedup();
    assert_eq!(called.len(), 1, "the arm calls the whole reader alone: {called:?}");
    assert!(!arm.contains(&format!("{mult}i64")), "a word hash in the `/` arm");
    assert!(!arm.contains("as i64).max("), "a `max` in the `/` arm");
}

/// The number arm is lean's: `while digit { end += 1 }`, a byte a turn (`RunForm::Bytes`), and
/// `.` and a digit asked at the run's end (`Bytewise::run_split`), then the second `while`; no
/// window of the bytes before and after for the `prev` and the next of every byte, no digit set's
/// word, its `.` known by its way; a token past 16 bytes the whole rule's call, cold, as lean's
/// `end - pos < 19 || parse()`; its leading zero asked of whether the first `while` ran a turn.
#[test]
fn a_numbers_arm_is_leans_loop() {
    let src = include_str!(concat!(env!("OUT_DIR"), "/tier1_aot.rs"));
    let f0 = &src[src.find("unsafe fn f0(").expect("the file loop")..];
    let m = f0.find(" { 34 => {").expect("the rules' match");
    let body = &f0[m..];
    let at = body.find("48..=57 => { break '").expect("the digits' arm") + "48..=57 => { break '".len();
    let label = &body[at..at + body[at..].find(';').expect("its label")];
    // The arm's code after its label's block.
    let open = f0.find(&format!("'{label}: {{")).expect("the label's block");
    let mut depth = 0;
    let mut close = open;
    for (i, ch) in f0[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    close = open + i;
                    break;
                }
            }
            _ => {}
        }
    }
    let rest = &f0[close..];
    // To where its whole rule's call joins its ways (the record written after, with the tail's).
    let call = rest.find("let t = unsafe { f").expect("the whole rule's call");
    let join = call + rest[call..].find("break '").expect("the call's join");
    let arm = &rest[..join];
    assert_eq!(arm.matches("loop {").count(), 2, "the number arm's `while digit` loops: {arm}");
    assert!(!arm.contains("vld1q_u8"), "a window in the number arm: {arm}");
    // Its one call, the whole rule's, cold.
    assert_eq!(arm.matches("unsafe { f").count(), 1, "the number arm's calls: {arm}");
    assert!(arm.contains("preserves_flags)) };\nlet t = unsafe { f"), "the whole rule's call not cold: {arm}");
    // A leading zero known from the first `while`'s turn (`way_lane`), inline: the long way's
    // call alone cold.
    assert_eq!(arm.matches("cold_path").count(), 1, "a cold test in the number arm: {arm}");
    // A digit asked as lean's `is_ascii_digit`, a subtract and a compare, not a set's word.
    let digits: u64 = (b'0'..=b'9').fold(0, |m, b| m | 1 << b);
    assert!(!arm.contains(&format!("{}i64", digits as i64)), "a digit set's word in the number arm: {arm}");
    // Its `.` known by the way it took (`split_counts`), not counted over a mask of the window.
    assert!(!arm.contains("vceqq_u8") && !arm.contains("count_ones"), "a count of the number's `.`: {arm}");
}

/// The loop's next start is the block's base and the starts' next bit, as lean's `base + q`:
/// not the token's end's block (`end & !63`), which chained each start to the token before it
/// (its byte, its table entry, its run), so short tokens ran one after another.
#[test]
fn the_next_start_is_the_blocks() {
    let src = include_str!(concat!(env!("OUT_DIR"), "/tier1_aot.rs"));
    let f0 = &src[src.find("unsafe fn f0(").expect("the file loop")..];
    let f0 = &f0[..f0[10..].find("\nunsafe fn ").map_or(f0.len(), |e| e + 10)];
    assert!(!f0.contains("(-64i64 as u64)"), "a start from a token's end's block");
}
