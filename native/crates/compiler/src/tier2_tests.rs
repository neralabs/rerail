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
