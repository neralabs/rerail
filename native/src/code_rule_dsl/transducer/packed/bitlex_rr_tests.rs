//! The `.rr` ladder (docs/parser-generators/24-rr-and-bitlex-plan.md §3): `.rr`, the rule
//! language, a construct at a time, each step a Bitlex file in `bitlex_ladder/rr/`, parsed with
//! no Rust between the bytes and the nodes.
//!
//! Each step is checked as the ladder's are (`bitlex_ladder_tests::check_ladder`): the reference
//! evaluator, the interpreters and Cranelift give the same nodes on every document, and each
//! step has the compiler's shape. And each document a step parses is parsed by the engine's
//! own `.rr` parser too (`logic::rr::tape`): the two trees must be equal, node for node (kind,
//! field, span, size), and the engine's must have no `ERROR`. A step's kinds and fields are named
//! as the engine's tape names them (`logic::rr::tape`).
//!
//! A step refuses what it does not have yet, so a document the engine parses and the step
//! refuses is no difference: it is counted.

use super::bitlex_ladder_tests::{Compiled, IdNode, check_ladder, steps_in};
use crate::code_rule_dsl::logic::rr;

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/code_rule_dsl/transducer/packed/bitlex_ladder/rr");

/// The bytes a mutation writes: `.rr`'s punctuation, quotes, comment openers and words.
const ALPHABET: &[u8] = b"().,:-=<>!\"`/*#_ \naZ9~;@{}\\";

/// Sample texts of the token kinds the steps' grammars name, with the edge cases of each.
const SAMPLES: &[(&str, &[&str])] = &[(
    "string",
    &["\"a\"", "\"\"", "\"x/y.rr\"", "\"a \\\" b\"", "\"\\\\\"", "\"two\nlines\"", "\"// not a comment\"", "\"é\"", "\"\\u{e9}\""],
), (
    "comment",
    &["// c", "//", "// a, (1) é\r", "//// x", "// #include \"y\"", "// /* x", "// \"x", "/* c */", "/**/", "/*/", "/* a\n b */", "/* // \" ` */", "/*** x **/"],
), (
    "code_literal",
    &["`a`", "``", "` `", "`\"`", "`//`", "`/*`", "`` a`b ``", "``x``", "``` a `` b ```", "`\\`", "`é`"],
)];

/// Documents for the rules a random sentence rarely meets, each with the first step that parses
/// it. Every step checks each one as it checks its sentences: the reference, the interpreters,
/// Cranelift and the engine agree, and each step from the one named on must parse it.
const CASES: &[(&str, &str)] = &[
    ("#include \"a\" // c\n#include \"b\"", "01"),
    // A setting: its words to the end of its line.
    (".mode block\n.scope tests false\n", "02"),
    (".mode", "02"),
    (".mode x", "02"),
    // A line that ends in `,` goes on; at the end of the file, a `,` ends it.
    (".scope dirs \"a\",\n  \"b\",\n\n  \"c\"\n.mode x", "02"),
    (".scope a,", "02"),
    (".scope a, // why\n b\n", "02"),
    // A comment at a line's end: in the setting when the line goes on, after it when not.
    (".mode x // c\n.scope y", "02"),
    (".mode x // c\n// d\n", "02"),
    // A string may span lines; the token after it, on its last line, is on a new line.
    (".description \"two\nlines\"\n.mode x", "02"),
    // The `,` after it is on a new line: a statement of its own, which the engine refuses.
    (".scope \"a\nb\", \"c\"", "never"),
    // The name on a line after the `.`: no arguments.
    (".\nmode\n.scope y", "02"),
    // Punctuation, the two-byte spellings from the left; numbers and identifiers.
    (".a :-> <=> =>= !=! <:- -> * ; ( ) [ ] @ | % ~ ^ & + / . :", "02"),
    (".a 1a 12 007 a1 _b #include \"x\"", "02"),
    ("#include \"a\" .mode x\n", "02"),
    // A declaration: `.decl`, one leaf; its columns and types; its tail to its `)`'s line's end.
    (".decl r(a: number, b: flat<x>) inline\n.decl s()\n", "03"),
    (".decl r(a: t<>) eqrel, b\n.mode x", "03"),
    (".decl r(\n  a: t,\n  b: u\n) x\n.mode z", "03"),
    // `decl` after a line break, or a comment (the leaf's text), after the `.`.
    (".\ndecl r(a: t)", "03"),
    (". // c\n decl r(a: t)", "03"),
    // A soft keyword: a word wherever the grammar does not take the keyword.
    (".mode decl\n.decl decl(decl: decl)", "03"),
    // A word a step reserves (a directive it does not parse yet) refuses the file; one that
    // only begins with it does not. `violation` is 9 bytes: two 8-byte compares.
    (".mode violations", "02"),
    (".mode violation", "10"),
    // Clauses: heads, a body of atoms; terms: numbers, strings, wildcards, variables, calls,
    // named arguments.
    ("r(x).\ns(1, \"a\", _) :- t(x), u().", "04"),
    ("r(x, y), s(y) :- t(x, f(y, g(z)), k: z, n: m(1)).", "04"),
    ("r() :- s(a: b: c).\n.mode x\nt(x) :- u(x).", "04"),
    // `_` is a word where no term can be: a head's relation. In a term it is a wildcard, so
    // `_(y)` is no call, and at a literal's start no atom: the engine refuses both.
    ("_(x) :- r(_, _z).", "04"),
    ("r(x) :- r(_(y)).", "never"),
    ("r(x) :- _(y).", "never"),
    // Comparisons: of a variable, a number, a string, a wildcard; and a call, on the right.
    ("r(x) :- s(x), x != 1, 2 < x, \"a\" = y, _ >= f(x), x <= y, z > w.", "04"),
    // A word whose next token decides it, across a comment and a line break.
    ("r(f // c\n (x), k\n: v, w // d\n).", "04"),
    // Regions, each hiding the others' openers; the first opener wins. `/*/` is a whole block
    // comment, as the engine reads it.
    ("r(x). /*/ s(y).", "05"),
    ("r(x) /* a \" b\n // c */ :- s(\"/* x\", \"// y\").", "05"),
    ("r(x). // a \" b /* c\ns(y).", "05"),
    ("r(`a`, `` b`c ``, ``` d `` e ```) :- s(`//`, `\"`, `/*`).", "05"),
    // A `` closes at the next ``, whatever is between.
    ("r(o) :- s(o, ``), t(o, ``).", "05"),
    // Open at the end of the file: the engine refuses it.
    ("r(x). /* x", "never"),
    ("r(\"x).", "never"),
    ("r(`x).", "never"),
    // A `` with no closing pair is the engine's empty literal, found by a look to the file's
    // end: the file goes to the engine.
    ("r(``).", "never"),
    // Negation, patterns: `!` before an atom, a pattern or a negation, never a comparison.
    ("r(x) :- !s(x), !!t(x), x ~ `a*`, !y ~ `b`, `c` ~ `d`, \"e\" ~ `f`.", "05"),
    ("r(x) :- !x = 1.", "never"),
    // A head's modal: before a word; before a `(` it is the relation.
    ("must r(x) :- s(x).", "05"),
    ("must r(x), may s(x) :- t(x).\nmust(x).\nmay (x) :- must(x).", "05"),
    ("must // c\n(x).\nmay /* d */ r(x).", "05"),
    // An extra between a word and the `(` that wraps it: never the node the wrap moves.
    ("r(x) :- s // c\n(x), t /* d */ (x).", "05"),
    ("r(n) :- n = count : { s(_) }.", "08"),
    ("r(-1).", "06"),
];

/// A node by names: kind, field ("" for none), start, end, size.
type Named = (String, String, usize, usize, usize);

/// The engine's tree of `src`, by names, and whether it holds an `ERROR`.
fn engine(src: &str) -> (Vec<Named>, bool) {
    let tape = rr::tape(src);
    let mut size = vec![1usize; tape.len()];
    for i in (0..tape.len()).rev() {
        if let Some(p) = tape[i].parent {
            size[p as usize] += size[i];
        }
    }
    let error = tape.iter().any(|n| n.is_error());
    let named = tape
        .iter()
        .zip(&size)
        .map(|(n, &s)| {
            let kind = if n.is_error() { "ERROR".to_owned() } else { rr::tape::KIND_NAMES[usize::from(n.kind)].to_owned() };
            (kind, rr::tape::FIELD_NAMES[usize::from(n.field)].to_owned(), n.start as usize, n.end as usize, s)
        })
        .collect();
    (named, error)
}

/// A step's nodes by names: its ids back to the names they were made from.
fn named(c: &Compiled, nodes: &[IdNode]) -> Vec<Named> {
    let names: std::collections::HashMap<u16, String> = c.ids.0.borrow().iter().map(|(n, &i)| (i, n.clone())).collect();
    nodes
        .iter()
        .map(|n| {
            let field = if n.1 == 0 { String::new() } else { names[&n.1].trim_start_matches("field ").to_owned() };
            (names[&n.0].clone(), field, n.3, n.4, n.5)
        })
        .collect()
}

/// The engine's tree against the step's, for a document the step parses.
fn against_the_engine(c: &Compiled, bytes: &[u8], want: &Option<Vec<IdNode>>) -> Option<String> {
    let nodes = want.as_ref()?;
    // A `.rr` file is text: the engine reads only UTF-8.
    let src = std::str::from_utf8(bytes).ok()?;
    let (theirs, error) = engine(src);
    let ours = named(c, nodes);
    if error {
        return Some(format!("the step parses what the engine does not:\n  engine {theirs:?}\n  step {ours:?}"));
    }
    if ours == theirs {
        return None;
    }
    let at = ours.iter().zip(&theirs).position(|(a, b)| a != b).unwrap_or(ours.len().min(theirs.len()));
    Some(format!(
        "the trees differ at node {at}: step {:?}, engine {:?}\n  engine {theirs:?}\n  step {ours:?}",
        ours.get(at),
        theirs.get(at)
    ))
}

/// Every step of the `.rr` ladder: the ladder's checks, and the engine's tree on every document
/// the step parses.
#[test]
fn rr_steps_parse_as_the_reference_and_the_engine() {
    let steps = steps_in(DIR);
    assert!(!steps.is_empty(), "no steps in {DIR}");
    let fixed: Vec<&str> = CASES.iter().map(|c| c.0).collect();
    let differ = check_ladder(&steps, ALPHABET, SAMPLES, &fixed, &|_, c, bytes, want| against_the_engine(c, bytes, want));
    assert!(differ.is_empty(), "{} steps differ:\n{}", differ.len(), differ.join("\n"));
}

/// Each case parses from the step that names it on, and the engine's tree has no `ERROR`.
#[test]
fn rr_cases_parse_from_their_steps() {
    let mut missing = Vec::new();
    for (name, source) in steps_in(DIR) {
        let c = super::bitlex_ladder_tests::compile_step(&name, &source);
        for &(doc, from) in CASES {
            if name.as_str() >= from && super::bitlex_ladder_tests::reference(&c, doc.as_bytes()).is_none() {
                missing.push(format!("{name}: {doc:?}: {}", super::bitlex_ladder_tests::refusal(&c, doc.as_bytes())));
            }
        }
    }
    assert!(missing.is_empty(), "cases a step refuses:\n{}", missing.join("\n"));
}

/// A document's tokens and parse at a step, for a look by hand: `RERAIL_RR_STEP` (a step's name,
/// or part of it) and `RERAIL_RR_DOC` (the document, with `\n` for a line break).
#[test]
#[ignore = "a look by hand: run by name with RERAIL_RR_STEP and RERAIL_RR_DOC"]
fn rr_look() {
    let (Ok(step), Ok(doc)) = (std::env::var("RERAIL_RR_STEP"), std::env::var("RERAIL_RR_DOC")) else { return };
    let doc = doc.replace("\\n", "\n");
    for (name, source) in steps_in(DIR).into_iter().filter(|s| s.0.contains(&step)) {
        let c = super::bitlex_ladder_tests::compile_step(&name, &source);
        let out = ::bitlex::eval::Ev::run(&c.stmts, doc.as_bytes(), &std::collections::HashMap::new()).expect("runs");
        eprintln!("{name}: errors {:?}", out.errors);
        for t in &out.tokens {
            eprintln!("  {:>4}..{:<4} {}{} {:?}", t.start, t.end, if t.mark { "mark " } else { "" }, t.kind, &doc[t.start..t.end]);
        }
        eprintln!("  reference: {:?}", super::bitlex_ladder_tests::refusal(&c, doc.as_bytes()));
        eprintln!("  engine: {:?}", engine(&doc).0);
    }
}
