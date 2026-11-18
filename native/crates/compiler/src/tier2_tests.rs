//! Tier 2 end to end in this crate: each Bitlex ladder step compiled (`bitlex_masks`),
//! assembled and verified (`rir_asm`), lowered (`native_ir`) and run two ways, by the
//! interpreters (tier 1's block program, then tier 2's words, `stream::run`) and by the
//! Cranelift code (`native_cpu_stream::Parser`). Both give the same nodes, or the same
//! refusal, for each document.
//!
//! The documents here are a fixed handful over the ladder's constructs. The engine's ladder
//! tests draw many more from each step's grammar and check them against the reference.

use crate::bitlex_masks::{self, FALLBACK, Symbols};
use crate::native_cpu_stream::Parser;
use crate::{native_ir, rir_asm, stream};
use std::cell::RefCell;
use std::collections::HashMap;

/// The ladder's files.
const DIR: &str = crate::ladder::DIR;

/// A kind or field id per name, in order of first use, as a grammar would number them.
#[derive(Default)]
struct Ids(RefCell<HashMap<String, u16>>);

impl Ids {
    fn of(&self, name: &str) -> u16 {
        let mut m = self.0.borrow_mut();
        let n = m.len() as u16 + 1;
        *m.entry(name.to_owned()).or_insert(n)
    }
}

/// Documents over the ladder's constructs, well formed and not: the empty file, keywords,
/// lists, records, texts with escapes, numbers, both kinds of comment, and cut-off input.
const DOCUMENTS: &[&str] = &[
    "",
    "vrai",
    "fausse",
    "nul",
    " vrai ",
    "vrai fausse",
    "(vrai;fausse;nul)",
    "((vrai);())",
    "(vrai;",
    "<#a# = vrai; #b# = (1;2)>",
    "#texte#",
    "#t^#x^^#",
    "#cut",
    "42",
    "~3,14e~2",
    "1,",
    "-- note\nvrai",
    "{- note -} vrai",
    "{- open",
    "{\"a\": [1, 2.5e3, true, null, \"x\\n\"]}",
    "[1, 2,]",
    "// c\n[1]",
];

/// Each step: the compiled words equal the interpreters on every document, node for node, or
/// refuse at the same word.
#[test]
fn every_ladder_step_compiles_here_and_equals_the_interpreters() {
    let mut steps: Vec<_> = std::fs::read_dir(DIR).expect("the ladder").map(|e| e.expect("an entry").path()).filter(|p| p.extension().is_some_and(|x| x == "bitlex")).collect();
    steps.sort();
    assert!(steps.len() >= 15, "{} steps", steps.len());
    let mut parsed = 0;
    for path in steps {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
        let source = std::fs::read_to_string(&path).expect("a step");
        let stmts = ::bitlex::syntax::parse(&source).unwrap_or_else(|e| panic!("{name}: parses: {e}"));
        let ids = Ids::default();
        let kind = |n: &str| Some(ids.of(n));
        let field = |n: &str| Some(ids.of(&format!("field {n}")));
        let symbols = Symbols { kind: &kind, field: &field };
        let compiled = bitlex_masks::compile(&stmts, &[FALLBACK], Some(&symbols)).unwrap_or_else(|e| panic!("{name}: compiles: {e}"));
        let text = compiled.text();
        let words = rir_asm::assemble(&text).unwrap_or_else(|e| panic!("{name}: assembles: {e}\n{text}"));
        let structure = native_ir::lower(&words, 0, 0).unwrap_or_else(|e| panic!("{name}: lowers: {e:?}"));
        let parser = Parser::new(&structure).unwrap_or_else(|e| panic!("{name}: Cranelift refuses: {e:?}"));
        let block = &words.block_programs[0];
        for doc in DOCUMENTS {
            let bytes = doc.as_bytes();
            // The nodes, or the word that refused (the compiled code says only that it refused).
            let pc = |r: Result<Vec<stream::Node>, stream::Refused>| r.map_err(|e| e.pc);
            let want = pc(stream::run(&words, bytes, &block.run(bytes)));
            parsed += usize::from(want.is_ok());
            assert_eq!(pc(parser.parse(bytes)), want, "{name}: {doc:?}");
        }
    }
    // Not refusals alone: many documents parse, so the nodes are compared too.
    assert!(parsed >= 30, "{parsed} documents parsed");
}

/// The crate's assembler takes a parser's program only: a matcher's rows are the engine's.
#[test]
fn the_crate_assembles_only_a_parsers_program() {
    assert!(rir_asm::assemble("program -\n  mode set=0 query=0\n  0000 STOP 0\n").is_ok());
    let matcher = "program -\n  mode set=0 query=0\n  label 0 s0\n  string 0 \"x\"\n  0000 EMIT L0\n";
    assert!(rir_asm::assemble(matcher).is_err(), "a matcher's words");
    assert!(rir_asm::assemble("program sha256:00\n  0000 STOP 0\n").is_err(), "a digest");
}

/// Shared arms (`native_cpu_stream::shared_runs`) on every ladder step: each place of a body
/// holds the body's words, runs do share, and refusals inside a place that is not its body's
/// first say that place's word (the equality above checks the word; this, that it happens
/// wherever such a place holds a word that can refuse).
#[test]
fn shared_arms_are_one_copy_and_refuse_at_their_own_word() {
    let mut steps: Vec<_> = std::fs::read_dir(DIR).expect("the ladder").map(|e| e.expect("an entry").path()).filter(|p| p.extension().is_some_and(|x| x == "bitlex")).collect();
    steps.sort();
    let (mut runs, mut bodies, mut inside, mut refusing) = (0, 0, 0, 0);
    for path in steps {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
        let stmts = ::bitlex::syntax::parse(&std::fs::read_to_string(&path).expect("a step")).expect("parses");
        let ids = Ids::default();
        let kind = |n: &str| Some(ids.of(n));
        let field = |n: &str| Some(ids.of(&format!("field {n}")));
        let symbols = Symbols { kind: &kind, field: &field };
        let compiled = bitlex_masks::compile(&stmts, &[FALLBACK], Some(&symbols)).expect("compiles");
        let words = rir_asm::assemble(&compiled.text()).expect("assembles");
        let structure = native_ir::lower(&words, 0, 0).expect("lowers");
        let pushes: Vec<u16> = structure.structural.iter().filter_map(|w| if let native_ir::StructuralOperation::Push { target, .. } = w.operation { Some(target) } else { None }).collect();
        let shared = crate::native_cpu_stream::shared_runs(&structure, &pushes);
        let ops = |a: usize, b: usize| structure.structural[a..b].iter().map(|w| w.operation).collect::<Vec<_>>();
        for (&s, &(b, ti)) in &shared.at {
            let body = &shared.bodies[b];
            let len = body.end - body.start;
            assert_eq!(ops(s, s + len), ops(body.start, body.end), "{name}: the place at {s} holds its body's words");
            assert!(matches!(structure.structural[s + len].operation, native_ir::StructuralOperation::Goto { target } if target == body.targets[ti]), "{name}: the place at {s} goes on to its own target");
        }
        // The words in a place past its body's first that can refuse: a keyword's (`WORD`), a
        // length's (`SPAN`).
        refusing += (0..shared.skipped.len()).filter(|&pc| shared.skipped[pc] && matches!(structure.structural[pc].operation, native_ir::StructuralOperation::Word { test: false, .. } | native_ir::StructuralOperation::Span { .. })).count();
        runs += shared.at.len();
        bodies += shared.bodies.len();
        // Every prefix of every document, each by the interpreters and the compiled code: the
        // refusals inside shared places, counted, give the same word.
        let parser = Parser::new(&structure).expect("Cranelift compiles");
        let block = &words.block_programs[0];
        for doc in DOCUMENTS {
            for cut in 0..=doc.len() {
                let bytes = &doc.as_bytes()[..cut];
                let want = stream::run(&words, bytes, &block.run(bytes)).map_err(|r| r.pc);
                assert_eq!(parser.parse(bytes).map_err(|r| r.pc), want, "{name}: {:?}", String::from_utf8_lossy(bytes));
                if let Err(pc) = want {
                    inside += usize::from([pc.wrapping_sub(1), pc].iter().any(|&pc| (pc as usize) < shared.skipped.len() && shared.skipped[pc as usize]));
                }
            }
        }
    }
    assert!(bodies < runs, "{runs} runs, {bodies} bodies: none shared");
    // Where such a place holds a word that can refuse, a document refuses there. The switches
    // name keywords (`ByteSwitch::words`), so on these steps none does: a token is refused at
    // its state's switch.
    assert!(inside > 0 || refusing == 0, "{refusing} words that can refuse in shared places, and no document refuses inside one");
}

/// JSON's number check is a token's check (decision 0398): tier 2 runs it over each number's
/// own bytes, as simdjson's stage 2 parses a number, and tier 1 makes none of it. On the CPU and
/// in the interpreter it refuses exactly what RFC 8259's grammar refuses, at every length: a
/// number of one window, one ending at a window's edge, and ones of two and three windows, whose
/// `fill`s and `prev`s carry from window to window.
#[test]
fn a_numbers_check_runs_over_its_tokens_bytes() {
    // RFC 8259: `-? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE] [+-]? [0-9]+)?`.
    fn rfc(s: &[u8]) -> bool {
        let digits = |s: &[u8], i: usize| s[i..].iter().take_while(|b| b.is_ascii_digit()).count();
        let mut i = usize::from(s.first() == Some(&b'-'));
        match s.get(i) {
            Some(b'0') => i += 1,
            Some(b'1'..=b'9') => i += digits(s, i),
            _ => return false,
        }
        if s.get(i) == Some(&b'.') {
            let d = digits(s, i + 1);
            if d == 0 {
                return false;
            }
            i += 1 + d;
        }
        if matches!(s.get(i), Some(b'e' | b'E')) {
            i += 1;
            if matches!(s.get(i), Some(b'+' | b'-')) {
                i += 1;
            }
            let d = digits(s, i);
            if d == 0 {
                return false;
            }
            i += d;
        }
        i == s.len()
    }
    let source = std::fs::read_to_string(format!("{DIR}/14-json.bitlex")).expect("the ladder's JSON");
    let c = crate::ladder::compile_step("14-json", &source);
    assert!(c.program.local_checks.iter().any(|k| k.token), "no token's check:\n{:?}", c.program.local_checks);
    let block = &c.program.block_programs[0];
    let ones = |n: usize| "1".repeat(n);
    let mut numbers: Vec<String> = ["0", "-0", "7", "-12", "1.5", "0.0001", "1e5", "1E+5", "1e-05", "-1.25e+300", "01", "1.", "-", "1e", "1e+", "--1", "1.2.3", "1.e5", "-01", "0x1", "1x", "2-", "1e5.0", "1ee5"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    for n in [62, 63, 64, 65, 66, 127, 128, 129, 130, 200] {
        numbers.push(ones(n));
        numbers.push(format!("-{}", ones(n - 1)));
        numbers.push(format!("{}x", ones(n - 1)));
        numbers.push(format!("{}.", ones(n - 1)));
        numbers.push(format!("{}-", ones(n - 1)));
        numbers.push(format!("0{}", ones(n - 1)));
        numbers.push(format!("1.{}", "2".repeat(n - 2)));
        numbers.push(format!("1.{}e", "2".repeat(n - 3)));
        numbers.push(format!("{}e+{}", ones(n / 2), "9".repeat(n - n / 2 - 2)));
        numbers.push(format!("{}e+{}.", ones(n / 2), "9".repeat(n - n / 2 - 3)));
    }
    let (mut accepted, mut refused) = (0, 0);
    for number in &numbers {
        for doc in [format!("[{number}]"), format!("{{\"k\": [1, {number}, 2]}}"), format!("{}{number}", " ".repeat(60))] {
            let bytes = doc.as_bytes();
            let pc = |r: Result<Vec<stream::Node>, stream::Refused>| r.map_err(|e| e.pc);
            let want = pc(stream::run(&c.program, bytes, &block.run(bytes)));
            assert_eq!(want.is_ok(), rfc(number.as_bytes()), "the interpreter: {doc:?}");
            assert_eq!(pc(c.parser.parse(bytes)), want, "the CPU: {doc:?}");
            if want.is_ok() { accepted += 1 } else { refused += 1 }
        }
    }
    assert!(accepted > 50 && refused > 50, "{accepted} accepted, {refused} refused");
}

/// JSON's `\u` checks are token's checks at each escape (decision 0399): tier 2 tests the
/// escape's `u` and the bytes after it, tier 1 makes none of it. The interpreter and the
/// CPU refuse exactly the strings a `\u` without four hex digits or a lone surrogate is in,
/// with the escape at every place of a block's last bytes, so its bytes cross into the next
/// word, and after escaped backslashes, where a `u` is no escape's.
#[test]
fn a_unicode_escapes_checks_run_at_its_u() {
    // Whether a string's text (between its quotes, no control bytes) is valid JSON.
    fn valid(s: &[u8]) -> bool {
        let hex4 = |i: usize| s.len() >= i + 4 && s[i..i + 4].iter().all(u8::is_ascii_hexdigit);
        let unit = |i: usize| u16::from_str_radix(std::str::from_utf8(&s[i..i + 4]).expect("hex"), 16).expect("hex");
        let mut i = 0;
        let mut want_low = false;
        while i < s.len() {
            if s[i] != b'\\' {
                if want_low {
                    return false;
                }
                i += 1;
                continue;
            }
            match s.get(i + 1) {
                Some(b'u') => {
                    if !hex4(i + 2) {
                        return false;
                    }
                    let u = unit(i + 2);
                    let (high, low) = ((0xd800..0xdc00).contains(&u), (0xdc00..0xe000).contains(&u));
                    if want_low != low || (want_low && high) {
                        return false;
                    }
                    want_low = high;
                    i += 6;
                }
                Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') if !want_low => i += 2,
                _ => return false,
            }
        }
        !want_low
    }
    let source = std::fs::read_to_string(format!("{DIR}/14-json.bitlex")).expect("the ladder's JSON");
    let c = crate::ladder::compile_step("14-json", &source);
    // One token's check past its token (the hex digits and the lone high, merged), and no mask's.
    assert_eq!(c.program.local_checks.iter().filter(|k| k.token && k.delay == 8).count(), 1, "{:?}", c.program.local_checks);
    assert!(c.program.local_checks.iter().all(|k| k.token), "{:?}", c.program.local_checks);
    let block = &c.program.block_programs[0];
    let texts = [
        r"A", r"éx", r"😀", r"😀", r"\u12", r"\u12G4", r"\uZZZZ", r"\u", r"\uD800", r"\uD800x", r"\uD800A", r"\uDC00",
        r"a\uDE00", r"\uD800\uD800", r"\\u12", r"\\A", r"\\\u12", r"\\\\uD800", r"\uD83D\n\uDE00", r"􏿿", r"\uD800\\uDC00",
    ];
    let (mut accepted, mut refused) = (0, 0);
    for text in texts {
        for pad in 50..70 {
            // The last: the file ends in the escape's bytes, an unclosed string (a `within`'s
            // test of the file's end).
            for (doc, closed) in [(format!("[\"{}{text}\"]", "a".repeat(pad)), true), (format!("{{\"{text}\": {}}}", "1".repeat(pad % 7 + 1)), true), (format!("\"{}{text}\"", " ".repeat(pad)), true), (format!("\"{}{text}", "b".repeat(pad)), false)] {
                let bytes = doc.as_bytes();
                let pc = |r: Result<Vec<stream::Node>, stream::Refused>| r.map_err(|e| e.pc);
                let want = pc(stream::run(&c.program, bytes, &block.run(bytes)));
                assert_eq!(want.is_ok(), closed && valid(text.as_bytes()), "the interpreter: {doc:?}");
                assert_eq!(pc(c.parser.parse(bytes)), want, "the CPU: {doc:?}");
                if want.is_ok() { accepted += 1 } else { refused += 1 }
            }
        }
    }
    assert!(accepted > 100 && refused > 100, "{accepted} accepted, {refused} refused");
}

/// Decision 0400, R1 to R3: JSON's and JSONC's tokens end at their gaps, with no extract of
/// their last bytes, and on every ladder step whose tokens do, the lasts and codes are the old
/// extract's. JSONC's comments among them: those ending in spaces, at a token, at the file's
/// end, and those the fallback takes (a comment opened inside another, a quote inside one).
#[test]
fn a_tokens_end_is_its_gaps_as_the_lasts_extract() {
    let mut docs: Vec<String> = DOCUMENTS.iter().map(|d| d.to_string()).collect();
    for pad in 0..70 {
        let a = "a".repeat(pad);
        let sp = " ".repeat(pad % 9);
        docs.extend([
            format!("[\"{a}\\n\\u00e9x  \", 12.5e{pad}, -0, true,null ,false]"),
            format!("{{ \"{a}\" :\t[ {} ] }}  ", "1".repeat(pad + 1)),
            format!("\"{a}\\\"\\\\ "),
            format!("[1{sp}"),
            format!("// c {sp}\n[1, /* {a} */ 2]\n"),
            format!("[1/*{a}*/,2//{sp}\n]//{a}"),
            format!("{{\"k\"/* {sp} */:/**/true}}/* {a} {sp}*/"),
            format!("[1, // {a} /* x\n2]"),
            format!("[1 /* {a} // */, 2]"),
            format!("[1 // \"{a}\n]"),
            format!("[1 /* {a} \" */]"),
            format!("[1]/* {a}"),
        ]);
    }
    let mut with = Vec::new();
    for (name, source) in crate::ladder::steps() {
        let c = crate::ladder::compile_step(&name, &source);
        let block = &c.program.block_programs[0];
        let Some(r) = block.records.as_ref().filter(|r| r.gap.is_some()) else { continue };
        with.push(name.clone());
        let native = crate::block_native::BlockNative::lower(block);
        assert!(native.ruled_lasts().is_some(), "{name}: the CPU makes no lasts");
        // The program with the lasts extracted, as before.
        bitlex_masks::NO_GAP_ENDS.set(true);
        let was = crate::ladder::compile_step(&name, &source);
        bitlex_masks::NO_GAP_ENDS.set(false);
        let old = &was.program.block_programs[0];
        assert!(old.records.as_ref().is_some_and(|r| r.gap.is_none()), "{name}: the oracle extracts the lasts");
        let (mut tokens, mut refused) = (0, 0);
        for doc in &docs {
            let bytes = doc.as_bytes();
            let (new, was) = (block.run(bytes), old.run(bytes));
            // A file the fallback takes has no records tier 2 reads.
            assert_eq!(new.totals, was.totals, "{name}: {doc:?}");
            if was.totals.iter().any(|&t| t != 0) {
                refused += 1;
                continue;
            }
            assert_eq!(new.positions[usize::from(r.lasts)], was.positions[usize::from(r.lasts)], "{name}: {doc:?}");
            assert_eq!(new.codes, was.codes, "{name}: {doc:?}");
            tokens += new.positions[0].len();
        }
        let json = name.starts_with("14-") || name.starts_with("15-");
        assert!(tokens > if json { 1000 } else { 0 } && (refused > 0 || !json), "{name}: {tokens} tokens, {refused} files refused");
    }
    for want in ["14-json", "15-jsonc"] {
        assert!(with.iter().any(|n| n.starts_with(want)), "{want}'s tokens do not end at their gaps: {with:?}");
    }
}

/// Decisions 0405 and 0406: the hot loop carries the bytes before its block in a register
/// (simdjson's `prev_input_block`), and its UTF-8 error as bytes ORed into a vector (simdjson's
/// `error`), reduced where the loop leaves.
/// - Its loop loads no byte before its block, and reduces one vector a block to a word (the
///   ASCII test): no error is made a mask or a word there.
/// - The CPU's tier 1 agrees with the interpreter on UTF-8 across the edges of full blocks,
///   where the carried chunk is the byte before, and in the first and last blocks (the edge
///   part's): files of five blocks.
#[test]
#[cfg(target_arch = "aarch64")]
fn the_hot_loop_carries_the_bytes_before_its_block() {
    for (name, source) in crate::ladder::steps().into_iter().filter(|(n, _)| n.starts_with("14-json") || n.starts_with("15-jsonc")) {
        let c = crate::ladder::compile_step(&name, &source);
        let hot = crate::clif_text::machine_loop(&c.parser.tier1.loops_disassembly());
        let before: Vec<&String> = hot.iter().filter(|l| l.starts_with("ldr q") && l.contains(", #-")).collect();
        assert!(before.is_empty(), "{name}: the hot loop loads bytes before its block: {before:?}");
        let reduced = hot.iter().filter(|l| l.starts_with("mov x") && l.ends_with(".d[1]")).count();
        assert_eq!(reduced, 1, "{name}: vectors reduced to words in the hot loop");
        let made = crate::block_native::BlockNative::lower(&c.program.block_programs[0]).cpu_unstored();
        let (mut docs, mut refused) = (0, 0);
        for seq in [&[0xc3u8, 0xa9][..], &[0xe2, 0x82, 0xac], &[0xf0, 0x9f, 0x98, 0x80], &[0xed, 0xa0, 0x80], &[0xe0, 0x80, 0x80], &[0xf4, 0x90, 0x80, 0x80], &[0xc0, 0x80], &[0xf5, 0x80], &[0x80], &[0xc3], &[0xe2, 0x82], &[0xc3, 0x80, 0x80]] {
            for edge in [20usize, 128, 192, 300] {
                for back in 0..5 {
                    let pad = edge - 2 - back;
                    let mut doc = format!("[\"{}", "a".repeat(pad)).into_bytes();
                    doc.extend(seq);
                    doc.extend(b"\", 1]");
                    doc.resize(320, b' ');
                    let interpreted = c.program.block_programs[0].run_files(&[&doc]).remove(0);
                    assert!(crate::ladder::tier1_agrees(&c.parser.tier1.extract(&doc), &interpreted, &made), "{name}: {seq:02x?} at {pad}");
                    docs += 1;
                    refused += usize::from(interpreted.totals.iter().any(|&t| t != 0));
                }
            }
        }
        assert!(0 < refused && refused < docs, "{name}: {refused} of {docs} refused");
    }
}

/// Decisions 0402, 0404 and 0405: JSON's UTF-8 rules of two bytes are byte-pair lookups on the
/// CPU, as simdjson's (`native_cpu_pairs`): the eight ORed in the check seven boxes of one pair
/// (simdjson's bits: `f4..ff` before `9x..bx` one, `f0` and `f5..ff` before `8x` one),
/// `TWO_CONTS` another, on the same two tables, this byte's a table of high nibbles (one lookup). The CPU's tier 1 still
/// agrees with the interpreter's: every lead byte before continuation bytes and others, complete
/// or cut short, continuations first or in runs, across the chunks' and the blocks' edges.
#[test]
fn utf8s_two_byte_rules_are_one_pair_lookup() {
    let (name, source) = crate::ladder::steps().into_iter().find(|(n, _)| n.starts_with("14-json")).expect("14-json");
    let c = crate::ladder::compile_step(&name, &source);
    let native = crate::block_native::BlockNative::lower(&c.program.block_programs[0]);
    let (_, stripped) = crate::native_cpu_block::host_stripped(&native).expect("the host strips the mark");
    let fused = crate::native_cpu_pairs::fused(&stripped).expect("the rules are pairs");
    let pairs: Vec<(u16, u16, u8)> = fused.insts.iter().filter_map(|i| if let crate::block_native::BlockInst::Pair { before, table, bits } = *i { Some((before, table, bits)) } else { None }).collect();
    let bits: Vec<u32> = pairs.iter().map(|p| p.2.count_ones()).collect();
    assert_eq!(bits, [7, 1], "{name}: the eight rules in seven boxes, then two continuations: {pairs:?}");
    assert!(pairs.iter().all(|p| (p.0, p.1) == (pairs[0].0, pairs[0].1)), "{name}: one pair of tables: {pairs:?}");
    assert_eq!(fused.tables[usize::from(pairs[0].1)].lo, [0xff; 16], "{name}: this byte's table is of high nibbles");
    let made = native.cpu_unstored();
    let (mut docs, mut refused) = (0, 0);
    for lead in [0xc2u8, 0xdf, 0xe0, 0xe1, 0xec, 0xed, 0xee, 0xef, 0xf0, 0xf1, 0xf3, 0xf4, 0xf5] {
        let len = if lead < 0xe0 { 2 } else if lead < 0xf0 { 3 } else { 4 };
        for next in (0x70u8..=0xc1).step_by(3).chain([0x8f, 0x90, 0x9f, 0xa0]) {
            for pad in [0usize, 1, 13, 14, 15, 29, 45, 60, 61, 62, 63, 69] {
                let mut doc = format!("[\"{}", "a".repeat(pad)).into_bytes();
                doc.extend([lead, next]);
                doc.extend(std::iter::repeat_n(0x80u8, len - 2));
                doc.extend(b"\", 1]");
                // A continuation more, a lead byte for the continuation, and the bytes first in
                // the file (where the byte before is none).
                let mut more = doc.clone();
                more.insert(pad + 4, 0x80);
                let mut lead_lead = doc.clone();
                lead_lead[pad + 3] = 0xc3;
                let first = [&[next, lead][..], b" 1"].concat();
                for d in [more, lead_lead, first] {
                    let (compiled, interpreted) = (c.parser.tier1.extract(&d), c.program.block_programs[0].run_files(&[&d]).remove(0));
                    assert!(crate::ladder::tier1_agrees(&compiled, &interpreted, &made), "{name}: {d:02x?}");
                }
                let (compiled, interpreted) = (c.parser.tier1.extract(&doc), c.program.block_programs[0].run_files(&[&doc]).remove(0));
                assert!(crate::ladder::tier1_agrees(&compiled, &interpreted, &made), "{name}: {doc:02x?}");
                refused += usize::from(interpreted.totals.iter().any(|&t| t != 0));
                docs += 1;
            }
        }
    }
    assert!(refused > 0 && refused < docs, "{refused} of {docs} refused");
}

/// Decision 0404: JSON's UTF-8 check in simdjson's form (`lookup4`, every rule of two bytes a
/// pair, `TWO_CONTS` against the lead bytes 2 and 3 back) refuses exactly the files its plain
/// form does: a lead byte needs its continuations, and a continuation needs its lead. Every
/// string of up to 3 bytes from the classes' edges, and of 4 from fewer, at the file's start
/// and inside a string.
#[test]
fn utf8_in_simdjsons_form_refuses_as_the_plain_form() {
    let source = std::fs::read_to_string(format!("{DIR}/14-json.bitlex")).expect("14-json");
    let at = source.find("error \"invalid UTF-8\"").expect("the check");
    let end = at + source[at..].find("\nerror ").expect("the next error");
    // The plain form: the lead bytes 1, 2 and 3 back say where a continuation is needed.
    let plain_check = r#"class never     = 0xc0 0xc1 0xf5..0xff
class lead234   = 0xc2..0xf4
class lead34    = 0xe0..0xf4
class lead4only = 0xf0..0xf4
mask needed     = prev(lead234, 1) | prev(lead34, 2) | prev(lead4only, 3)
error "invalid UTF-8" when never | (needed ^ cont)
                         | (prev(e0, 1) & below_a0) | (prev(ed, 1) & from_a0)
                         | (prev(f0, 1) & below_90) | (prev(f4, 1) & from_90)"#;
    let plain = format!("{}{plain_check}{}", &source[..at], &source[end..]);
    let (a, b) = (crate::ladder::compile_step("14-json", &plain), crate::ladder::compile_step("14-json", &source));
    let (pa, pb) = (&a.program.block_programs[0], &b.program.block_programs[0]);
    let alphabet = [0x41u8, 0x7f, 0x80, 0x8f, 0x90, 0x9f, 0xa0, 0xbf, 0xc0, 0xc1, 0xc2, 0xdf, 0xe0, 0xe1, 0xec, 0xed, 0xee, 0xef, 0xf0, 0xf1, 0xf3, 0xf4, 0xf5, 0xff];
    let (mut n, mut refused, mut differ) = (0usize, 0usize, Vec::new());
    let mut seqs: Vec<Vec<u8>> = vec![Vec::new()];
    let small = [0x41u8, 0x80, 0xa0, 0xbf, 0xc2, 0xe0, 0xed, 0xf0, 0xf4];
    for round in 0..4 {
        let from: &[u8] = if round < 3 { &alphabet } else { &small };
        let base: Vec<Vec<u8>> = if round < 3 { seqs.clone() } else { seqs.iter().filter(|s| s.iter().all(|c| small.contains(c))).cloned().collect() };
        seqs = base.iter().flat_map(|s| from.iter().map(move |&c| [s.clone(), vec![c]].concat())).collect();
        for s in &seqs {
            for doc in [s.clone(), [b"\"".to_vec(), s.clone(), b"\"".to_vec()].concat()] {
                let (x, y) = (pa.run_files(&[&doc]).remove(0), pb.run_files(&[&doc]).remove(0));
                let (rx, ry) = (x.totals.iter().any(|&t| t != 0), y.totals.iter().any(|&t| t != 0));
                n += 1;
                refused += usize::from(rx);
                if rx != ry && differ.len() < 10 {
                    differ.push(format!("{doc:02x?}: old {rx} new {ry}"));
                }
            }
        }
    }
    assert!(differ.is_empty() && refused > 0 && refused < n, "{n} documents, {refused} refused; differ: {differ:#?}");
}
