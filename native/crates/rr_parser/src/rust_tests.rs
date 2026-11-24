//! The `.rerail` parser as Rust (`rerail-rr-aot`, written by the Bitlex compiler from the grammar, decision 0560)
//! against the JIT's compile of the same grammar: the files are the compiler's output, and tier 1's records are the
//! interpreter's.

use crate::{Compiled, SAMPLES, compiled};

/// `rerail_rr_aot`'s `tier1.rs` and `tier2.rs` are the Bitlex compiler's output for the grammar, in the compile
/// `compiled` makes (its codes and ids are the JIT's); `RERAIL_BLESS=1` writes them again.
#[test]
fn rust_is_current() {
    let c = compiled().expect("the grammar compiles");
    let tier1 = c.lexer_rust.clone().unwrap_or_else(|e| panic!("tier 1 as Rust: {e}"));
    let tier2 = c.rust.clone().unwrap_or_else(|e| panic!("tier 2 as one loop: {e}"));
    for (file, rust) in [("tier1.rs", tier1), ("tier2.rs", tier2)] {
        let path = format!("{}/../rr_aot/src/{file}", env!("CARGO_MANIFEST_DIR"));
        if std::env::var_os("RERAIL_BLESS").is_some() {
            std::fs::write(&path, &rust).expect("writes");
        }
        assert!(std::fs::read_to_string(&path).is_ok_and(|have| have == rust), "rr_aot's {file} is not the compiler's: RERAIL_BLESS=1");
    }
}

/// A tape's nodes, in order: each one's number in the `.rerail` table, start, end and size.
type Nodes = Vec<(u32, u64, u64, usize)>;

/// The parser as Rust's tape of `buf[..n]` (`rerail_rr_aot::parse`, `buf` padded), in format 16 through the lean
/// parser's sink; or the byte it refused the file at.
fn aot_tape(buf: &[u8], n: usize) -> Result<crate::Tape, u32> {
    rerail_rr_aot::parse(buf, n, &mut rerail_rr_syntax::lean::Sink16::new(false, true)).map(|t| crate::written(t.expect("a format 16 tape").tape).expect("format 16"))
}

/// A format 16 tape's nodes, read in place.
fn tape_nodes(t: &crate::Tape) -> Nodes {
    use rerail_tape_format::tape::Tape as _;
    let r = t.reader(rerail_rr_syntax::tape::lang()).expect("a .rerail tape");
    (0..r.len()).map(|i| (r.code(i), r.start(i), r.end(i), r.subtree_end(i) - i)).collect()
}

/// Where the parser as Rust's tape is not the JIT's: whether it refuses the file, and where neither does, the first
/// node that differs.
fn tape_differs(c: &Compiled, text: &str) -> Option<String> {
    let theirs = c.image(text.as_bytes()).ok().map(|t| tape_nodes(&t));
    let ours = aot_tape(&rerail_rr_aot::padded(text.as_bytes()), text.len()).ok().map(|t| tape_nodes(&t));
    match (theirs, ours) {
        (None, None) => None,
        (Some(_), None) => Some("the JIT parses it, the parser as Rust refuses it".into()),
        (None, Some(_)) => Some("the parser as Rust parses it, the JIT refuses it".into()),
        (Some(a), Some(b)) => {
            let k = (0..a.len().max(b.len())).find(|&k| a.get(k) != b.get(k))?;
            let at = |n: Option<&(u32, u64, u64, usize)>| n.map_or(String::new(), |n| text[n.1 as usize..].chars().take(16).collect::<String>().replace('\n', "⏎"));
            Some(format!("node {k} of {}/{}: the JIT {:x?} `{}`, the parser as Rust {:x?} `{}`", a.len(), b.len(), a.get(k), at(a.get(k)), b.get(k), at(b.get(k))))
        }
    }
}

/// The marks no record of tier 1's holds (`tier1::UNRECORDED`, decision 0623): those tier 2 asks of the bytes or of the
/// tokens, by their bits.
fn tier2_asked_marks() -> u32 {
    let tier1 = std::fs::read_to_string(format!("{}/../rr_aot/src/tier1.rs", env!("CARGO_MANIFEST_DIR"))).expect("reads");
    let Some(at) = tier1.find("pub const UNRECORDED: u32 = 0x") else { return 0 };
    let rest = &tier1[at + 30..];
    u32::from_str_radix(&rest[..rest.find(';').expect("the constant's end")], 16).expect("the bits")
}

/// The mark bits of the hidden marks only refused (`refuse M in L`): the lowering checks their local token tests in
/// tier 2, in the refusing levels (`bitlex_masks`, `kind_checks`), so the interpreter's records lack those bits; tier
/// 1 as Rust tests them in the token loop and records them. Each bit is the one tier 1 as Rust gives the mark.
fn tier2_checked_marks() -> u32 {
    let stmts = ::bitlex::syntax::parse(crate::GRAMMAR).expect("parses");
    let tier1 = std::fs::read_to_string(format!("{}/../rr_aot/src/tier1.rs", env!("CARGO_MANIFEST_DIR"))).expect("reads");
    let mut out = 0;
    for s in &stmts {
        let ::bitlex::syntax::Stmt::Tree { body, .. } = s else { continue };
        for t in body {
            let ::bitlex::syntax::Stmt::Refuse { mark, .. } = t else { continue };
            let at = tier1.find(&format!("// `{mark}`\n")).unwrap_or_else(|| panic!("mark {mark} in tier1.rs"));
            let rest = &tier1[at..];
            let b = rest.find("marks |= 1 << ").map(|i| &rest[i + 14..]).and_then(|r| r[..r.find(';')?].parse::<u32>().ok()).expect("the mark's bit");
            out |= 1 << b;
        }
    }
    out
}

/// A file nested too deep ends in a refusal, never in a stack that grows without end: tier 1 counts no brackets, and
/// tier 2's one loop refuses a level past its stack's depth (decision 0623). Nested a little past the grammar's 127, it
/// may parse where the JIT refuses it; both refuse it far past.
#[test]
fn deep_nesting_ends_in_a_refusal() {
    for n in [128, 200, 3_000, 100_000] {
        for text in [format!("a(X) :- b(X), X = {}1{}.\n", "[".repeat(n), "]".repeat(n)), format!(".policy {}{}\n", "{\"a\": ".repeat(n), "}".repeat(n))] {
            let got = aot_tape(&rerail_rr_aot::padded(text.as_bytes()), text.len());
            if n >= 3_000 {
                assert!(got.is_err(), "{n} deep: refused");
            }
        }
    }
}

/// Where tier 1 as Rust (`rerail_rr_aot::lex`) is not the interpreter's tier 1: whether it sends the file to the
/// engine, and where neither does, each token's first and last byte and code. In the bits of the marks tier 2 checks
/// (`tier2_checked_marks`), the Rust's are the interpreter's and those tier 2 tests.
fn tier1_differs(c: &Compiled, text: &str) -> Option<String> {
    // The marks tier 2 asks of the bytes or the tokens (decision 0623) are in no record of tier 1's: the tapes test them.
    let asked = tier2_asked_marks() << 16;
    let late = tier2_checked_marks() << 16 & !asked;
    let run = c.program.block_programs[0].run(text.as_bytes());
    let lx = rerail_rr_aot::lex(&rerail_rr_aot::padded(text.as_bytes()), text.len());
    let refused = run.totals.first().is_some_and(|&t| t != 0);
    // An escape the lowering makes a check of tier 2's at a token (`pending_checks`): the interpreter's tier 1 does
    // not refuse there, tier 1 as Rust does; either way the file is refused.
    if lx.refused && !refused && c.image(text.as_bytes()).is_err() {
        return None;
    }
    if refused != lx.refused {
        return Some(format!("refused: the interpreter {refused}, tier 1 as Rust {}", lx.refused));
    }
    if refused {
        return None;
    }
    let (starts, lasts) = (&run.positions[0], &run.positions[1]);
    for k in 0..starts.len().max(lx.recs.len()) {
        let theirs = (starts.get(k), lasts.get(k), run.codes.get(k));
        let rec = lx.recs.get(k);
        let ours = (rec.map(|r| &r.start), rec.map(|r| &r.last), rec.map(|r| &r.code));
        let kept = |c: Option<&u32>| c.map(|&c| c & !late & !asked);
        let late_ok = run.codes.get(k).zip(ours.2).is_none_or(|(&t, &o)| t & late & !o == 0);
        if (theirs.0, theirs.1, kept(theirs.2)) != (ours.0, ours.1, kept(ours.2)) || !late_ok {
            let at = |s: Option<&u32>| s.map_or(String::new(), |&s| text[s as usize..].chars().take(16).collect::<String>().replace('\n', "⏎"));
            return Some(format!("record {k}: the interpreter {theirs:x?} `{}`, tier 1 as Rust {ours:x?} `{}`", at(theirs.0), at(ours.0)));
        }
    }
    None
}

/// Files that reach tier 1's rules one at a time: the depth's count at and past its bound, the floats (`toggle`'s
/// parity of a chain's dots, and a float the file ends with: a mark's `else escape`), the regions (an empty
/// backtick literal: a region's `else`), the marks.
fn cases() -> Vec<String> {
    let mut out: Vec<String> = SAMPLES.iter().map(|s| (*s).to_owned()).collect();
    // A region whose closer is blocks past its opener: tier 1's one pass reads the token again as stage 1 makes them
    // (decision 0619), and a `` `` `` with its closer there is no empty literal (`else e_t2`).
    for n in [60, 200, 700] {
        out.push(format!("a(b) :- r(c, ``{}``).\n", "x".repeat(n)));
        out.push(format!("a(\"{}\").\n", "y".repeat(n)));
        out.push(format!("/* {} */ a(1).\n", "z".repeat(n)));
    }
    // Past the grammar's 127 the JIT refuses; the parser as Rust refuses past its own stack's depth
    // (`deep_nesting_ends_in_a_refusal`).
    for n in [126, 127] {
        out.push(format!("a(X) :- b(X), X = {}1{}.\n", "[".repeat(n), "]".repeat(n)));
        out.push(format!(".policy {}{}\n", "{\"a\": ".repeat(n), "}".repeat(n)));
    }
    for t in [
        "a(X) :- b(X), X = 1.5.\n",
        "a(X) :- b(X), X = 1.5",
        "a(X) :- b(X), X = 1.2.3.4.\n",
        "a(X) :- X = 12.\n",
        "a(1.5, 2.25, 3).\n",
        "a(X) :- X = 1.",
        "a(\"s\\u00e9\\u{1F600}\", `x`, ``y``, ```z```).\n",
        "a(````q````).\n",
        "a(\"open\n",
        "/* open\n",
        "a(X) :- b(X). // line\n/* block */ c(Y) :- d(Y).\n",
        "#include \"x.rerail\"\n.decl a(x: number)\n",
        ".type T = <a: number>\n.input a, b(x)\n.output c\n",
        ".violation v(x: number) code \"C1\" tag {t} \"message\" fix safe r \"d\" suppress tool \"r\", \"s\" over {o}\n",
        "a(X) :- count(Y) : { b(Y) } = X, min(Z) by (W) : { c(Z, W) } > 1.\n",
        // An aggregate's call read again inside it (`min(…) : {…}`) takes the aggregate's word (`remember`): its own
        // again after an aggregate in its arguments has set its own.
        "a(X) :- X = min(Y) : { b(Y) }.\n",
        "a(X) :- X = min(max(Y) : { c(Y) }) : { b(Y) }.\n",
        "a(X) :- X = sum(Y, mean(Z) : { c(Z) }) : { b(Y) }.\n",
        "a(X) :- min(max(Y) : { c(Y) }, Z) : { b(Z) } = X.\n",
        // A second argument there: its `,` dropped, a dead node the reading reaches, refused (`drop`).
        "a(X) :- X = sum(Y, Z) : { b(Y) }.\n",
        "a(X) :- !b(X), (c(X); d(X)), X ~ `p`.\n",
        "a(1234567890123456789).\n",
        "a(123456789012345678).\n",
        "\u{0c}a(1).\n",
        // A string's escapes a JSON document refuses (`json_bad`, refused in its levels only): in a clause, and in a
        // policy's document.
        "a(\"\\q\", 1).\n",
        "a(\"\\u12\", 1).\n",
        "a(\"\\u{1F600}\").\n",
        ".policy {\"a\": \"\\q\"}\n",
        ".policy {\"a\": \"\\u{1F600}\"}\n",
        ".policy {\"a\": \"x\", \"b\": [1, {\"c\": \"\\q\"}]}\n",
        ".policy {\"a\": \"x\"}\na(\"\\q\").\n",
        // `else e_t2 bytes 2`: two backticks with no closer after them, a token of their two bytes.
        "a(b) :- r(c, ``).\n",
        "a(b) :- r(o, ` `), r(c, ``), r(d, ``x``).\n",
    ] {
        out.push(t.to_owned());
    }
    out
}

#[test]
fn tier1s_records_are_the_interpreters() {
    let c = compiled().expect("the grammar compiles");
    let (mut same, mut refused) = (0, 0);
    for (k, t) in cases().iter().enumerate() {
        if let Some(d) = tier1_differs(c, t) {
            panic!("case {k} ({:?}): {d}", &t[..t.len().min(60)]);
        }
        if rerail_rr_aot::lex(&rerail_rr_aot::padded(t.as_bytes()), t.len()).refused { refused += 1 } else { same += 1 }
    }
    eprintln!("{same} files' records the interpreter's, {refused} refused by both");
    assert!(same > 0 && refused > 0);
}

#[test]
fn tapes_are_the_jits() {
    let c = compiled().expect("the grammar compiles");
    let (mut same, mut refused) = (0, 0);
    // Every case that differs, each named, so that a run counts them all.
    let mut differ = Vec::new();
    let cases = cases();
    for (k, t) in cases.iter().enumerate() {
        if let Some(d) = tape_differs(c, t) {
            eprintln!("differs: case {k} ({:?}): {d}", &t[..t.len().min(60)]);
            differ.push(format!("case {k} ({:?}): {d}", &t[..t.len().min(60)]));
            continue;
        }
        if c.image(t.as_bytes()).is_ok() { same += 1 } else { refused += 1 }
    }
    assert!(differ.is_empty(), "{} of {} cases differ; the first: {}", differ.len(), cases.len(), differ[0]);
    eprintln!("{same} tapes the JIT's, {refused} refused by both");
    assert!(same > 0 && refused > 0);
}

/// `tier1s_records_are_the_interpreters` over the repository's `.rerail` files.
#[test]
#[ignore = "the corpus: run by name"]
fn corpus_tier1s_records_are_the_interpreters() {
    let c = compiled().expect("the grammar compiles");
    let files = crate::tests::corpus();
    for (k, t) in files.iter().enumerate() {
        if let Some(d) = tier1_differs(c, t) {
            if let Ok(dir) = std::env::var("RERAIL_RR_DIFFERS") {
                std::fs::write(format!("{dir}/differs.rerail"), t).expect("writes");
            }
            panic!("file {k}: {d}");
        }
    }
    eprintln!("{} files", files.len());
}

/// `tapes_are_the_jits` over the repository's `.rerail` files. `RERAIL_RR_DIFFERS=<dir>`: the first file that
/// differs written there.
#[test]
#[ignore = "the corpus: run by name"]
fn corpus_tapes_are_the_jits() {
    let c = compiled().expect("the grammar compiles");
    let files = crate::tests::corpus();
    let mut parsed = 0;
    // Every file that differs, each named, so that a run counts them all.
    let mut differ = Vec::new();
    for (k, t) in files.iter().enumerate() {
        if let Some(d) = tape_differs(c, t) {
            if differ.is_empty()
                && let Ok(dir) = std::env::var("RERAIL_RR_DIFFERS")
            {
                std::fs::write(format!("{dir}/differs.rerail"), t).expect("writes");
            }
            eprintln!("differs: file {k}: {d}");
            differ.push(format!("file {k}: {d}"));
            continue;
        }
        parsed += usize::from(c.image(t.as_bytes()).is_ok());
    }
    assert!(differ.is_empty(), "{} of {} files differ; the first: {}", differ.len(), files.len(), differ[0]);
    eprintln!("{} files, {parsed} parsed", files.len());
}

/// CPU time over the corpus (this thread's, the least of `RR_ROUNDS` rounds, 7 by default), the parsers in turns each
/// round: the parser as Rust (`rerail_rr_aot`), tier 1 alone (`lex`) and the whole (`parse`, tier 2 the difference);
/// the JIT (`Compiled::image`); and the engine's hand-written reader (`engine_tape`), for scale. Each from the
/// text, the buffers padded before. Build optimized (`--profile profiling`) and run alone, at nice 19; the load is
/// printed.
#[test]
#[ignore = "a benchmark: run by hand"]
fn rr_aot_against_jit() {
    let c = compiled().expect("the grammar compiles");
    let rounds: usize = std::env::var("RR_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(7);
    let files = crate::tests::corpus();
    let bufs: Vec<Vec<u8>> = files.iter().map(|t| rerail_rr_aot::padded(t.as_bytes())).collect();
    let bytes: usize = files.iter().map(String::len).sum();
    let time = |f: &dyn Fn() -> usize| {
        let t = rerail_prof::thread_cpu();
        std::hint::black_box(f());
        rerail_prof::thread_cpu() - t
    };
    let lex = || bufs.iter().zip(&files).map(|(b, t)| rerail_rr_aot::lex(b, t.len()).recs.len()).sum::<usize>();
    let aot = || bufs.iter().zip(&files).map(|(b, t)| aot_tape(b, t.len()).map_or(0, |c| c.len())).sum::<usize>();
    let jit = || files.iter().map(|t| c.image(t.as_bytes()).map_or(0, |i| i.len())).sum::<usize>();
    let engine = || files.iter().map(|t| crate::engine_tape(t).len()).sum::<usize>();
    // The JIT's tier 1 alone: its batch made and called, as `image` makes it.
    let jit_lex = || {
        files
            .iter()
            .map(|t| {
                let mut batch = rerail_cpu_block::native_cpu_block::Batch::new(&c.parser.tier1, &[t.as_bytes()]);
                batch.call(&c.parser.tier1);
                batch.counts.len()
            })
            .sum::<usize>()
    };
    // The lean parser (`rr_syntax::lean`, hand-written Rust): the shape the generated Rust is to have.
    let lean_lex = || files.iter().map(|t| crate::rr::lean::token_count(t)).sum::<usize>();
    let lean = || files.iter().map(|t| crate::rr::lean::tape(t).map_or(0, |t| t.len())).sum::<usize>();
    let mut best = [std::time::Duration::MAX; 7];
    for _ in 0..rounds {
        for (k, f) in [&lex as &dyn Fn() -> usize, &aot, &jit, &engine, &jit_lex, &lean_lex, &lean].into_iter().enumerate() {
            best[k] = best[k].min(time(f));
        }
    }
    let refused = |f: &dyn Fn(&str, &[u8]) -> bool| files.iter().zip(&bufs).filter(|(t, b)| f(t, b)).count();
    let (aot_refused, jit_refused) = (refused(&|t, b| aot_tape(b, t.len()).is_err()), refused(&|t, _| c.image(t.as_bytes()).is_err()));
    let mb = bytes as f64 / 1e6;
    let row = |name: &str, d: std::time::Duration| eprintln!("  {name:<44} {:>9.3} ms  {:>8.1} MB/s  {:>6.2} ns/byte", d.as_secs_f64() * 1e3, mb / d.as_secs_f64(), d.as_secs_f64() * 1e9 / bytes as f64);
    eprintln!("{} files, {:.2} MB; refused: the parser as Rust {aot_refused}, the JIT {jit_refused}; least of {rounds} rounds", files.len(), mb);
    row("the parser as Rust, tier 1 (lex)", best[0]);
    row("the parser as Rust, tier 2 (parse - lex)", best[1].saturating_sub(best[0]));
    row("the parser as Rust, whole (parse)", best[1]);
    row("the JIT, tier 1 (the batch)", best[4]);
    row("the JIT, tier 2 (image - batch)", best[2].saturating_sub(best[4]));
    row("the JIT, whole (image)", best[2]);
    row("the lean parser, its lexer (token_count)", best[5]);
    row("the lean parser, its parse (tape - lexer)", best[6].saturating_sub(best[5]));
    row("the lean parser, whole (tape)", best[6]);
    row("the engine's hand-written reader", best[3]);
    let load = std::process::Command::new("uptime").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()).unwrap_or_default();
    eprintln!("  load: {load}");
}
