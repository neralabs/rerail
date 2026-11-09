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
    // Aggregates: values, keys, separators; at a literal's start; the words as names elsewhere.
    ("r(n) :- n = sum v by k sep \",\" : { t(k, v) }, m = max x + 1 : { u(x); w(x) }.", "08"),
    ("r(n) :- count : { s(_) } = 1, min(a, b) > 1, sum x : { t(x) } > 2, n = join s by (k, j) : { u(s, k, j) }.", "08"),
    (".decl r(max: number, count: t)\nr(count, min) :- s(sum), count = 1, mean(x), max - 1 > join.", "08"),
    ("r(n) :- n = sum strlen(x) - strlen(a) : { t(x, a), !u(a) }.", "08"),
    // `count` in a value takes its `:`; a named argument in a value is no value.
    ("r(n) :- n = sum count : { s(_) } : { t(_) }.", "08"),
    ("r(n) :- n = sum f(k: v) : { s(_) }.", "never"),
    // `min (…) : { … }`, an aggregate the engine tells from a call after its `)`: to the engine.
    ("r(n) :- n = min (a + b) : { s(a, b) }.", "never"),
    ("r(-1).", "06"),
    // Arithmetic: precedence, left to right; `-` and a number one leaf, across an extra too;
    // unary minus over a primary; parentheses; functor calls.
    ("r(x) :- s(y), x = y * 2 + -1, z = -(y - @f(x)) % 3.", "06"),
    ("r(x) :- x = a - b - c, y = a + b * c - d / e % f, z = (a + b) * (c - d).", "06"),
    ("r(- 1, - // c\n 2, -x, --1, -(1), - - x, -@f(1)).", "06"),
    ("r(x) :- x + 1 > y, -x < 2, @f(x, -2) = 1, 1 + 2 ~ `a`, !x * 2 ~ `b`.", "06"),
    ("r(k: v + 1, a * b: c) :- s(f(x) + g(y) * 2).", "06"),
    // Disjunctions and groups; an atom or a call, by the token after it.
    ("r(x) :- s(x); t(x), u(x); v(x).", "07"),
    ("r(x) :- s(x), (t(x); !(u(x), x > f(x) + 1)), !(v(x)).", "07"),
    ("r(x) :- f(x) > 1, g(x) * 2 + 1 = h(y), k(x) ~ `a`, !m(x) ~ `b`, n(x).", "07"),
    ("r(x) :- ((s(x); t(x)); (u(x))).", "07"),
    // A parenthesized term at a literal's start: the engine tells it from a group by going
    // back after its `)`. The file goes to the engine.
    ("r(x) :- (x + 1) > 1.", "never"),
    // `.violation`: parts in any order, continuation lines, the words as names elsewhere.
    (".violation v(at: loc, c: symbol) code {c} in \"E[0-9]+\" tag {at} \"m {c}\"\nfix safe v_fix \"a fix\"\nsuppress ruff \"E501\", \"W291\" over {at}\noptions { \"a\": [1, {\"b\": 2}] }\nr(x) :- s(x).", "09"),
    (".violation v(at: loc) code \"X1\" \"m\"\n.violation w(at: loc) \"n\" fix safe \"d\" fix f \"e\"\n.decl code(fix: number, tag: t)", "09"),
    ("r(code, tag, fix, safe, suppress, over, options, in, violation) :- code(x), fix = 1.", "09"),
    (".violation\nv(at: loc) tag { t } code { c } \"m\" // c\n/* d */ suppress t \"r\"", "09"),
    // `fix`, `suppress`, `options` starting a clause right after a `.violation`, and `over` a
    // fact after a `suppress` line: the engine reads the word only before what follows it.
    (".violation v(at: loc) \"m\"\nfix(x) :- s(x).", "15"),
    (".violation v(at: loc) \"m\"\nsuppress(x) :- s(x).\n.violation w(at: loc) \"m\"\noptions(x).", "15"),
    (".violation v(at: loc) \"m\"\nsuppress t(x) :- s(x).", "never"),
    (".violation v(at: loc) \"m\"\nsuppress t \"r\"\nover(x).\nfix(1).", "15"),
    (".violation v(at: loc) \"m\" fix /* c */ f // d\n \"e\" suppress t /* c */ \"r\" over /* c */ {at}\noptions // c\n{}\nfix(y).", "15"),
    (".violation v(at: loc) \"m\"\nfix safe \"d\"\nfix safe s \"e\"\nfix \"x\".", "never"),
    (".violation v(at: loc) \"m\"\noptions = 1.", "never"),
    // A column in braces that is no single name: one `raw_text` leaf, nested braces counted.
    (".violation v(at: loc) code {at.code} tag { } \"m\"", "16"),
    (".violation v(at: loc) code { /* c */ at } tag {a {b} c} \"m\" suppress t \"r\" over {x y}\n.violation w(at: loc) code {at} \"m\" suppress t \"r\" over { at }", "16"),
    (".violation v(at: loc) code {at \"m\"", "never"),
    // JSON objects: terms, setting arguments; edits.
    ("r({\"a\": {\"b\": [1, 2]}}) :- s(x, {}), {\"c\": 1} = x => e(x, \"y\"), f({}).", "09"),
    (".budget * by code { \"A\": 3, \"B\": 4 }\n.budget * {\n  \"total\": 0\n}\nr(x).", "09"),
    ("r(n) :- n = sum {\"a\": 1} : { s(_) }, !{} ~ `x`.", "09"),
    // A JSON object over lines ends its setting's line: the engine reads lines from its `{`.
    (".s {\n} x\nr(x).", "never"),
    // An atom before `=>`: the atom's level takes `=` (a call's comparison), not `=>`, which
    // its caller takes when the level ends.
    ("m(s) :- f(s) => r(a, `$sp`).\nm(s) :- f(s), g(s) => r(a) , q(b).", "09"),
    // A comment after a setting's JSON object, before its line's end.
    (".budget * { \"total\": 0 }    // c\n.mode block", "09"),
    // Policy documents: JSON, as the engine's reader takes it; else `.policy` is a setting.
    (".policy {\"format\": \"v1\", \"id\": \"p\", \"revision\": 2, \"rules\": [{\"a\": -1, \"b\": [true, false, null, 0, -0]}, []], \"e\": {}}\n.policy \"id\" revision 1\nr(x).", "10"),
    (".policy {\n  \"a\": \"x\\\\y \\/ \\b\\f\\n\\r\\t\\\"\"\n} // c\n.\npolicy {} r(x).", "10"),
    (".policy\n.policy x {\"a\": 1}\n.policy [1]", "10"),
    // What JSON refuses: the engine makes the document an error; to it.
    (".policy {{POLICY}}", "never"),
    (".policy {\"a\": 01}", "never"),
    (".policy {\"a\": - 1}", "never"),
    (".policy {\"a\": \"x\\q\"}", "never"),
    (".policy {\"a\": \"two\nlines\"}", "never"),
    (".policy {\"a\": 1 // c\n}", "never"),
    (".policy {\"a\": [1, {\"b\": /* c */ 2}]}", "never"),
    (".policy {\"a\": 1e5}", "never"),
    (".policy {\"a\": [1,]}", "never"),
    (".policy {\"a\": 1, }", "never"),
    (".policy {\"a\" 1}", "never"),
    (".policy {\"a\": tru}", "never"),
    (".policy /* c */ {\"a\": 1}", "never"),
    // The other directives: all of `.rr`.
    (".type t = /* c */ a\n.input r(IO=x /* d */, f=\"y\")\n.functor f(x: number): symbol stateful // e\n.edit e(a: node)\n.pragma \"a\" \"b\"\n.lattice l<> { A -> @f, B -> 1 + x }\n.input a, b(x)\n.output c", "11"),
    // An empty raw text: no leaf, and its field on the node made last.
    (".type t\n.input r()\n.decl x(a: t)", "11"),
    (".output\n.decl a(x: t)\n.input must(x), b\n.input r((a), b(c)), s(())", "11"),
    (".type a = \"x\ny\"\nr(x).\n.type s <: number // c\n.pragma\n.pragma \"a\"", "11"),
    // The rest of a `.type`'s line is the line its name ends on: a token after a string over
    // two lines starts the next statement.
    (".type a = \"x\ny\" z\nr(x).", "never"),
    (".lattice l<b> { }\n.lattice m<> { A -> @f(x) + 1, B -> -2 * x, C -> count : { s(_) } }", "11"),
    (".functor f(x: number): t<u> stateful\n.functor g(): t<> x `y`\n.decl r(x: t) `lit` 1", "11"),
    // A line's end met in a nested level that ends there (a return type): its caller's.
    (".functor dot(a: symbol): symbol    // c\n.functor e(a: symbol): t<u> // d\nr(x).", "11"),
    // What the engine refuses by the token after it: an I/O directive with no relation and no
    // `.` after it; a functor named, not called, but before a `,` or a `}`; a `<` after a
    // return type that is not its own.
    (".output", "never"),
    (".input\n#include \"x\"", "never"),
    (".lattice l<> { A -> @f + 1 }", "never"),
    ("r(x) :- s(@f, x).", "never"),
    (".functor g(): t < 3", "never"),
    // A `(` at a literal's start: a group, or a term that an operator, a comparison or a `~`
    // after its `)` shows it was; groups in it terms too, an atom a call.
    ("a(x) :- b(o), (o - n) * 10 > o.\np(x) :- ((a(x))) + 1 > 2.\np(x) :- ((o)) ~ `p`, (b(x) ; c(x)), !(d(x)), ((a(x))).", "12"),
    ("p(x) :- (o) > 1, (a(x)) = b, (count : { s(_) }) > 2, !(o) - 1 ~ `p`, ((o - n) > 1), ((a(x)) + 1 > 2).", "12"),
    ("p(x) :- ( // c\n o - n) * 2 > 1, (a(x) /* d */) + 1 > 2, (-o) < 2, ((o)) * (n) > 1.", "12"),
    ("p(x) :- count : { (o - 1) > 2 } = n, max x : { (s(x)) } > 1, !((a(x))).", "12"),
    // What the engine refuses: a term where a literal goes; a group of more than one literal, a
    // disjunction or an atom before what only a term takes; a negated comparison.
    ("p(x) :- (o).", "never"),
    ("p(x) :- ((o), a(x)).", "never"),
    ("p(x) :- (o) - 1.", "never"),
    ("p(x) :- (a(x)) ~ `p`.", "never"),
    ("p(x) :- (a(x), b(x)) + 1 > 2.", "never"),
    ("p(x) :- (a(x) ; b(x)) > 1.", "never"),
    ("p(x) :- !(o) > 1.", "never"),
    // Floats: a `.` between digits, the digits after it the fraction; a `.` after a fraction
    // is no float's.
    (".mode 1.5\nr(x) :- s(x, y), y > 0.10, x = -2.5 * y, z = - 3.25.", "13"),
    (".scope 1.2.3\n.mode 1.2.3.4\nr(x) :- x = 1.2.", "13"),
    ("r(x) :- x = 1.05, y = 007.5, z = 12345678901234567890.5, (a - 1.5) * 2.0 > b, c = -0.5.", "13"),
    (".policy {\"a\": 1.5, \"b\": -0.25, \"c\": [1.0]}", "13"),
    (".lattice l<b> { A -> 1.5 * x }\n.functor f(x: number): t 2.5\nr(x) :- count : { s(_) } > 1.5, max v : { t(v) } < -2.25.", "13"),
    ("r(x) :- x = 1..5.", "never"),
    ("r(x) :- a = 1.2.3.4.", "never"),
    ("r(x) :- w = 1.5e3.", "never"),
    // `\u` in a string: the forms that are code points by their bytes.
    ("r(x) :- x = \"\\u212a\", y = \"\\u{1F600}\", z = \"\\u{41}\", w = \"a\\u017fb\\u0130\", v = \"\\u{10FFFF}\", u = \"\\u{FFFFF}\", t = \"\\\\u1234\".", "14"),
    (".policy {\"a\": \"\\u00e9\", \"b\": \"x\"}", "14"),
    ("r(x) :- x = \"\\uD800\".", "never"),
    ("r(x) :- x = \"\\u{D800}\", y = \"\\u{110000}\".", "never"),
    (".policy {\"a\": \"\\u{41}\"}", "never"),
    // The same bytes outside JSON: no refusal.
    ("r(x) :- x = a -b, s(\"a\\qb\", \"two\nlines\", 007) // c\n.", "06"),
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

/// `refuse M in L`: the compiler takes it only where a refused mark is refused between the
/// level's first token and its last, as the reference refuses it: a hidden mark, nested levels,
/// each level they call refusing it too, no level that may end and go on.
#[test]
fn a_refused_mark_is_checked() {
    let source = std::fs::read_to_string(format!("{DIR}/10-policy.bitlex")).expect("step 10");
    assert_eq!(super::bitlex_ladder_tests::compile_error(&source), None);
    let refuse = "refuse json_bad in json_document json_inner_object json_inner_array";
    for (from, to, why) in [
        (refuse, "refuse json_bad in json_document json_inner_object", "calls one that does not"),
        (refuse, "refuse json_bad in json_document json_member json_inner_object json_inner_array", "no nested level"),
        (refuse, "refuse json_bad in source_file json_document json_inner_object json_inner_array", "the root"),
        ("hidden line newline json_bad", "hidden line newline", "no hidden mark"),
        ("merge json_object ']'", "merge json_object ']' [merge json_object '.']", "may end only after its last token"),
    ] {
        assert_eq!(source.matches(from).count(), 1, "{from}");
        let e = super::bitlex_ladder_tests::compile_error(&source.replace(from, to));
        assert!(e.as_deref().is_some_and(|e| e.contains(why)), "{to}: {e:?}");
    }
}

/// `reread R`: the compiler takes a reading only by a name declared once, and gives its rows to
/// the program (`RETAG m=3` names one; `provisional` fields the program checks at `ACCEPT`).
#[test]
fn a_reading_is_checked() {
    let source = std::fs::read_to_string(format!("{DIR}/12-parenthesized.bitlex")).expect("step 12");
    assert_eq!(super::bitlex_ladder_tests::compile_error(&source), None);
    for (from, to, why) in [
        ("reread parenthesized_strict pattern_tail\n  provisional", "reread nowhere pattern_tail\n  provisional", "no reading of that name"),
        ("reading parenthesized_strict = ", "reading parenthesized = ", "two of that name"),
        ("conjunction gone, atom as call", "conjunction gone, conjunction gone, atom as call", "a kind twice"),
    ] {
        assert_eq!(source.matches(from).count(), 1, "{from}");
        let e = super::bitlex_ladder_tests::compile_error(&source.replace(from, to));
        assert!(e.as_deref().is_some_and(|e| e.contains(why)), "{to}: {e:?}");
    }
    let c = super::bitlex_ladder_tests::compile_step("12", &source);
    assert_eq!(c.program.readings.maps.len(), 2);
    assert_eq!(c.program.readings.provisional, [c.ids.of("field bare")]);
    let rereads = c.native.structural.iter().filter(|b| matches!(b.operation, super::native_ir::StructuralOperation::Reread { .. })).count();
    assert!(rereads > 0, "the words reread");
}

/// A document's tokens and parse at a step, for a look by hand: `RERAIL_RR_STEP` (a step's name,
/// or part of it) and `RERAIL_RR_DOC` (the document, with `\n` for a line break).
#[test]
#[ignore = "a look by hand: run by name with RERAIL_RR_STEP and RERAIL_RR_DOC"]
fn rr_look() {
    let (Ok(step), Ok(doc)) = (std::env::var("RERAIL_RR_STEP"), std::env::var("RERAIL_RR_DOC")) else { return };
    // `@path`: the document is that file.
    let doc = match doc.strip_prefix('@') {
        Some(path) => std::fs::read_to_string(path).expect("reads the file"),
        None => doc.replace("\\n", "\n"),
    };
    for (name, source) in steps_in(DIR).into_iter().filter(|s| s.0.contains(&step)) {
        let c = super::bitlex_ladder_tests::compile_step(&name, &source);
        let out = ::bitlex::eval::Ev::run(&c.stmts, doc.as_bytes(), &std::collections::HashMap::new()).expect("runs");
        eprintln!("{name}: errors {:?}", out.errors);
        if let Some(Err(e)) = out.tree.as_ref().map(|t| ::bitlex::tree::nodes(t, &out.tokens, doc.len())) {
            let at: usize = e.split(" at byte ").nth(1).and_then(|r| r.split(|ch: char| !ch.is_ascii_digit()).next()).and_then(|n| n.parse().ok()).unwrap_or(0);
            eprintln!("  tree: {e}: {:?}", &doc[at..(at + 200).min(doc.len())]);
        }
        let long = doc.len() > 2000;
        for t in out.tokens.iter().filter(|_| !long) {
            eprintln!("  {:>4}..{:<4} {}{} {:?}", t.start, t.end, if t.mark { "mark " } else { "" }, t.kind, &doc[t.start..t.end]);
        }
        eprintln!("  reference: {:?}", super::bitlex_ladder_tests::refusal(&c, doc.as_bytes()));
        let compiled = c.parser.parse(doc.as_bytes());
        eprintln!("  compiled: {:?}", compiled.as_ref().map(|n| n.len()));
        // Where the compiled tree leaves the reference's: the nodes around it.
        if let (Ok(nodes), Some(want)) = (&compiled, super::bitlex_ladder_tests::reference(&c, doc.as_bytes())) {
            let (ours, theirs) = (named(&c, &super::bitlex_ladder_tests::as_ids(nodes.clone())), named(&c, &want));
            // By kind, field and span: a size differs first at the nodes around the place.
            let spans = |n: &Named| (n.0.clone(), n.1.clone(), n.2, n.3);
            if let Some(at) = ours.iter().zip(&theirs).position(|(a, b)| spans(a) != spans(b)) {
                let from = at.saturating_sub(3);
                eprintln!("  compiled leaves the reference at node {at}:\n    compiled {:?}\n    reference {:?}", &ours[from..(at + 4).min(ours.len())], &theirs[from..(at + 4).min(theirs.len())]);
                let n = &theirs[at];
                eprintln!("  text: {:?}", &doc[n.2.saturating_sub(60)..(n.3 + 60).min(doc.len())]);
            }
        }
        let theirs = engine(&doc).0;
        if let Some(nodes) = super::bitlex_ladder_tests::reference(&c, doc.as_bytes()) {
            let ours = named(&c, &nodes);
            eprintln!("  reference tree: {ours:?}");
            let compiled = compiled.as_ref().ok().map(|n| named(&c, &super::bitlex_ladder_tests::as_ids(n.clone())));
            eprintln!("  the engine's tree: {}; the compiled tree the reference's: {}", ours == theirs, compiled.as_ref() == Some(&ours));
            if let Some(at) = ours.iter().zip(&theirs).position(|(a, b)| a != b) {
                eprintln!("  the reference leaves the engine at node {at}: {:?} / {:?}", ours.get(at), theirs.get(at));
            }
        }
        eprintln!("  engine: {theirs:?}");
    }
}

/// Every `.rr` file of the repository through a step's compiled path (`RERAIL_RR_STEP`, the
/// last step by default), against the engine's tree: the files it parses as the engine does,
/// those it parses differently or the engine refuses (each a bug), and those it sends to the
/// engine, by the reference's reason. Run by name.
#[test]
#[ignore = "the corpus: run by name"]
fn rr_corpus() {
    let steps = steps_in(DIR);
    let want = std::env::var("RERAIL_RR_STEP").ok();
    let (name, source) = steps.iter().rev().find(|s| want.as_ref().is_none_or(|w| s.0.contains(w.as_str()))).expect("a step");
    let c = super::bitlex_ladder_tests::compile_step(name, source);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("the repository").to_path_buf();
    let mut files = Vec::new();
    let mut work = vec![root];
    while let Some(dir) = work.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let skip = p.file_name().is_some_and(|n| [".git", "target", "node_modules"].contains(&n.to_string_lossy().as_ref()));
            if p.is_dir() && !skip {
                work.push(p);
            } else if p.extension().is_some_and(|x| x == "rr") {
                files.push(p);
            }
        }
    }
    files.sort();
    let (mut same, mut bytes_parsed) = (0usize, 0usize);
    let (mut differ, mut refused_by_engine) = (Vec::new(), Vec::new());
    let mut reasons: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut fell = Vec::new();
    for f in &files {
        let bytes = std::fs::read(f).expect("reads");
        let Ok(src) = std::str::from_utf8(&bytes) else { continue };
        match c.parser.parse(&bytes) {
            Ok(nodes) => {
                let ours = named(&c, &super::bitlex_ladder_tests::as_ids(nodes));
                let (theirs, error) = engine(src);
                if error {
                    refused_by_engine.push(f.display().to_string());
                } else if ours == theirs {
                    same += 1;
                    bytes_parsed += bytes.len();
                } else {
                    let at = ours.iter().zip(&theirs).position(|(a, b)| a != b).unwrap_or(ours.len().min(theirs.len()));
                    differ.push(format!("{}: node {at}: step {:?}, engine {:?}", f.display(), ours.get(at), theirs.get(at)));
                }
            }
            Err(_) => {
                let why = super::bitlex_ladder_tests::refusal(&c, &bytes);
                let mut why = why.split(" (byte").next().unwrap_or("").to_owned();
                // The grammar's refusal: the token it stops at, and the statement's first word.
                if why == "the token" {
                    why = stopped_at(&c, &bytes).unwrap_or(why);
                }
                if std::env::var("RERAIL_RR_FELL").is_ok_and(|w| why.contains(&w)) {
                    eprintln!("  fell: {why}: {}", f.display());
                }
                *reasons.entry(why.clone()).or_default() += 1;
                fell.push((why, f.display().to_string()));
            }
        }
    }
    eprintln!("{name}: {} files: {same} parsed as the engine does ({bytes_parsed} bytes), {} to the engine, {} differ, {} the engine refuses", files.len(), fell.len(), differ.len(), refused_by_engine.len());
    let mut by: Vec<(&String, &usize)> = reasons.iter().collect();
    by.sort_by(|a, b| b.1.cmp(a.1));
    for (why, n) in by {
        let first = fell.iter().find(|f| &f.0 == why).map_or("", |f| f.1.as_str());
        eprintln!("  {n:>5} {why}   e.g. {first}");
    }
    for d in differ.iter().take(20) {
        eprintln!("  DIFFERS {d}");
    }
    for d in refused_by_engine.iter().take(20) {
        eprintln!("  ENGINE REFUSES {d}");
    }
    assert!(differ.is_empty() && refused_by_engine.is_empty(), "{} differ, {} the engine refuses", differ.len(), refused_by_engine.len());
}

/// Where the reference's grammar stops on `bytes`: the token's text and the first token of the
/// statement it is in (a `.`-directive's word, or the token at the line's start).
fn stopped_at(c: &Compiled, bytes: &[u8]) -> Option<String> {
    let out = ::bitlex::eval::Ev::run(&c.stmts, bytes, &std::collections::HashMap::new()).ok()?;
    let e = ::bitlex::tree::nodes(out.tree.as_ref()?, &out.tokens, bytes.len()).err()?;
    let at: usize = e.split(" at byte ").nth(1)?.split(|ch: char| !ch.is_ascii_digit()).next()?.parse().ok()?;
    let t = out.tokens.iter().find(|t| t.start >= at && !t.mark)?;
    let text = String::from_utf8_lossy(&bytes[t.start..t.end.min(t.start + 12)]).into_owned();
    let line_start = bytes[..t.start].iter().rposition(|&b| b == b'\n').map_or(0, |p| p + 1);
    let first: String = String::from_utf8_lossy(&bytes[line_start..]).split_whitespace().next().unwrap_or("").chars().take(12).collect();
    let directive = if first.starts_with('.') { first } else { String::new() };
    let _ = directive;
    let after = out.tokens.iter().filter(|u| u.start >= t.end && !u.mark && u.kind != "comment").map(|u| String::from_utf8_lossy(&bytes[u.start..u.end.min(u.start + 12)]).into_owned()).next().unwrap_or_default();
    Some(format!("stops at a statement starting {} {text:?} {after:?}", t.kind))
}
