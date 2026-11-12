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
        let pushes: Vec<u16> = structure.structural.iter().filter_map(|w| if let native_ir::StructuralOperation::Push { target } = w.operation { Some(target) } else { None }).collect();
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
