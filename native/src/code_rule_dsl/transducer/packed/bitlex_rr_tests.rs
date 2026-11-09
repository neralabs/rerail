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
    &["// c", "//", "// a, (1) é\r", "//// x", "// #include \"y\"", "// /* x", "// \"x"],
)];

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
    let differ = check_ladder(&steps, ALPHABET, SAMPLES, &|_, c, bytes, want| against_the_engine(c, bytes, want));
    assert!(differ.is_empty(), "{} steps differ:\n{}", differ.len(), differ.join("\n"));
}
