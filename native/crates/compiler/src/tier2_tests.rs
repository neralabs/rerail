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

/// A document whose tape does not fit 2-byte columns (70,000 blanks after its first separator: a
/// later node's offset from its block's base past 64 KiB) is read again by the wide tier 2 (`Parser::wide`, at `Widths::WIDE`): its nodes
/// are the interpreter's, and its image is the writer's: offsets at 4 bytes, the rest at 2.
#[test]
fn a_tape_past_two_byte_columns_is_read_at_four() {
    let mut steps: Vec<_> = std::fs::read_dir(DIR).expect("the ladder").map(|e| e.expect("an entry").path()).filter(|p| p.extension().is_some_and(|x| x == "bitlex")).collect();
    steps.sort();
    let mut wide = 0;
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
        let parser = Parser::new(&structure).expect("Cranelift compiles");
        let block = &words.block_programs[0];
        for doc in DOCUMENTS {
            let Some(at) = doc.find([';', ',']) else { continue };
            let big = format!("{}{}{}", &doc[..=at], " ".repeat(70_000), &doc[at + 1..]);
            let bytes = big.as_bytes();
            let Ok(want) = stream::run(&words, bytes, &block.run(bytes)) else { continue };
            let image = parser.image(bytes).expect("an image");
            assert_eq!(image, stream::image(&want, &words.readings.extras, structure.tag, bytes), "{name}: {doc:?} with the blanks: the writer's image");
            assert_eq!(parser.parse(bytes).map_err(|e| e.pc), Ok(want), "{name}: {doc:?} with the blanks");
            wide += usize::from(image[rerail_tape_format::header::WIDTHS + 1] == 4);
        }
    }
    assert!(wide >= 10, "{wide} documents written at 4-byte offsets");
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
