//! `.rerail`'s grammar (`rr_parser/rr.bitlex`, docs/parser-generators/24-rr-and-bitlex-plan.md §3), parsed with no Rust
//! between the bytes and the nodes.
//!
//! It is checked as JSON's grammars are (`ladder::check_ladder`): the reference evaluator, the interpreters and
//! Cranelift give the same nodes on every document, and it has the compiler's shape. And each document it parses is
//! parsed by the engine's own `.rerail` parser too (`rerail_rr_syntax::tape`): the two trees must be equal, node for node
//! (kind, field, span, size), and the engine's must have no `ERROR`. Its kinds and fields are named as the engine's
//! tape names them (`rerail_rr_syntax::tape`).
//!
//! A document the engine parses and the grammar refuses (it goes to the engine) is no difference: it is counted.

use rerail_cpu_stream::ladder::{Compiled, IdNode, check_ladder};
use rerail_rr_syntax as rr;

/// The grammar, by name.
fn grammar() -> Vec<(String, String)> {
    vec![("rr".to_owned(), include_str!("../../rr_parser/rr.bitlex").to_owned())]
}

/// The bytes a mutation writes: `.rerail`'s punctuation, quotes, comment openers and words.
const ALPHABET: &[u8] = b"().,:-=<>!\"`/*#_ \naZ9~;@{}\\";

/// Sample texts of the token kinds the grammar names, with the edge cases of each.
const SAMPLES: &[(&str, &[&str])] = &[(
    "string",
    &["\"a\"", "\"\"", "\"x/y.rerail\"", "\"a \\\" b\"", "\"\\\\\"", "\"two\nlines\"", "\"// not a comment\"", "\"é\"", "\"\\u{e9}\""],
), (
    "comment",
    &["// c", "//", "// a, (1) é\r", "//// x", "// #include \"y\"", "// /* x", "// \"x", "/* c */", "/**/", "/*/", "/* a\n b */", "/* // \" ` */", "/*** x **/"],
), (
    "code_literal",
    &["`a`", "``", "` `", "`\"`", "`//`", "`/*`", "`` a`b ``", "``x``", "``` a `` b ```", "`\\`", "`é`"],
)];

/// Documents for the rules a random sentence rarely meets, each marked `parses` or `never` (refused, so it goes to
/// the engine). Each is checked as a sentence is: the reference, the interpreters, Cranelift and the engine agree,
/// and one marked `parses` must parse.
const CASES: &[(&str, &str)] = &[
    ("#include \"a\" // c\n#include \"b\"", "parses"),
    // A setting: its words to the end of its line.
    (".mode block\n.scope tests false\n", "parses"),
    (".mode", "parses"),
    (".mode x", "parses"),
    // A line that ends in `,` goes on; at the end of the file, a `,` ends it.
    (".scope dirs \"a\",\n  \"b\",\n\n  \"c\"\n.mode x", "parses"),
    (".scope a,", "parses"),
    (".scope a, // why\n b\n", "parses"),
    // A comment at a line's end: in the setting when the line goes on, after it when not.
    (".mode x // c\n.scope y", "parses"),
    (".mode x // c\n// d\n", "parses"),
    // A string may span lines; the token after it, on its last line, is on a new line.
    (".description \"two\nlines\"\n.mode x", "parses"),
    // The `,` after it is on a new line: a statement of its own, which the engine refuses.
    (".scope \"a\nb\", \"c\"", "never"),
    // The name on a line after the `.`: no arguments.
    (".\nmode\n.scope y", "parses"),
    // Punctuation, the two-byte spellings from the left; numbers and identifiers.
    (".a :-> <=> =>= !=! <:- -> * ; ( ) [ ] @ | % ~ ^ & + / . :", "parses"),
    (".a 1a 12 007 a1 _b #include \"x\"", "parses"),
    ("#include \"a\" .mode x\n", "parses"),
    // A declaration: `.decl`, one leaf; its columns and types; its tail to its `)`'s line's end.
    (".decl r(a: number, b: flat<x>) inline\n.decl s()\n", "parses"),
    (".decl r(a: t<>) eqrel, b\n.mode x", "parses"),
    (".decl r(\n  a: t,\n  b: u\n) x\n.mode z", "parses"),
    // `decl` after a line break, or a comment (the leaf's text), after the `.`.
    (".\ndecl r(a: t)", "parses"),
    (". // c\n decl r(a: t)", "parses"),
    // A soft keyword: a word wherever the grammar does not take the keyword.
    (".mode decl\n.decl decl(decl: decl)", "parses"),
    // A reserved word refuses the file; one that only begins with it does not. `violation` is 9 bytes: two 8-byte compares.
    (".mode violations", "parses"),
    (".mode violation", "parses"),
    // Clauses: heads, a body of atoms; terms: numbers, strings, wildcards, variables, calls,
    // named arguments.
    ("r(x).\ns(1, \"a\", _) :- t(x), u().", "parses"),
    ("r(x, y), s(y) :- t(x, f(y, g(z)), k: z, n: m(1)).", "parses"),
    ("r() :- s(a: b: c).\n.mode x\nt(x) :- u(x).", "parses"),
    // `_` is a word where no term can be: a head's relation. In a term it is a wildcard, so
    // `_(y)` is no call, and at a literal's start no atom: the engine refuses both.
    ("_(x) :- r(_, _z).", "parses"),
    ("r(x) :- r(_(y)).", "never"),
    ("r(x) :- _(y).", "never"),
    // Comparisons: of a variable, a number, a string, a wildcard; and a call, on the right.
    ("r(x) :- s(x), x != 1, 2 < x, \"a\" = y, _ >= f(x), x <= y, z > w.", "parses"),
    // A word whose next token decides it, across a comment and a line break.
    ("r(f // c\n (x), k\n: v, w // d\n).", "parses"),
    // Regions, each hiding the others' openers; the first opener wins. `/*/` is a whole block
    // comment, as the engine reads it.
    ("r(x). /*/ s(y).", "parses"),
    ("r(x) /* a \" b\n // c */ :- s(\"/* x\", \"// y\").", "parses"),
    ("r(x). // a \" b /* c\ns(y).", "parses"),
    ("r(`a`, `` b`c ``, ``` d `` e ```) :- s(`//`, `\"`, `/*`).", "parses"),
    // A `` closes at the next ``, whatever is between.
    ("r(o) :- s(o, ``), t(o, ``).", "parses"),
    // Open at the end of the file: the engine refuses it.
    ("r(x). /* x", "never"),
    ("r(\"x).", "never"),
    ("r(`x).", "never"),
    // A `` with no closing pair is the engine's empty literal, found by a look to the file's
    // end: the file goes to the engine.
    ("r(``).", "never"),
    // Negation, patterns: `!` before an atom, a pattern or a negation, never a comparison.
    ("r(x) :- !s(x), !!t(x), x ~ `a*`, !y ~ `b`, `c` ~ `d`, \"e\" ~ `f`.", "parses"),
    ("r(x) :- !x = 1.", "never"),
    // A head's modal: before a word; before a `(` it is the relation.
    ("must r(x) :- s(x).", "parses"),
    ("must r(x), may s(x) :- t(x).\nmust(x).\nmay (x) :- must(x).", "parses"),
    ("must // c\n(x).\nmay /* d */ r(x).", "parses"),
    // An extra between a word and the `(` that wraps it: never the node the wrap moves.
    ("r(x) :- s // c\n(x), t /* d */ (x).", "parses"),
    ("r(n) :- n = count : { s(_) }.", "parses"),
    // Aggregates: values, keys, separators; at a literal's start; the words as names elsewhere.
    ("r(n) :- n = sum v by k sep \",\" : { t(k, v) }, m = max x + 1 : { u(x); w(x) }.", "parses"),
    ("r(n) :- count : { s(_) } = 1, min(a, b) > 1, sum x : { t(x) } > 2, n = join s by (k, j) : { u(s, k, j) }.", "parses"),
    (".decl r(max: number, count: t)\nr(count, min) :- s(sum), count = 1, mean(x), max - 1 > join.", "parses"),
    ("r(n) :- n = sum strlen(x) - strlen(a) : { t(x, a), !u(a) }.", "parses"),
    // `count` in a value takes its `:`; a named argument in a value is no value.
    ("r(n) :- n = sum count : { s(_) } : { t(_) }.", "parses"),
    ("r(n) :- n = sum f(k: v) : { s(_) }.", "never"),
    // `min (…) : { … }`, an aggregate the engine tells from a call after its `)`: to the engine.
    // `min (…) : {…}`: the call read again as the aggregate's value in parentheses.
    ("r(n) :- n = min (a + b) : { s(a, b) }.", "parses"),
    ("r(n) :- n = max(a) : { s(a) }, m = sum(1, 2), k = join (f(a)) : { s(a) } + min(b).", "parses"),
    ("r(n) :- n = sum (a) : { s(a) ; t(a) }, min(n).", "parses"),
    // Two terms, none, a named argument: the engine refuses them.
    ("r(n) :- n = min (a, b) : { s(a, b) }.", "never"),
    ("r(n) :- n = min() : { s(a) }.", "never"),
    ("r(n) :- n = min(x: 1) : { s(a) }.", "never"),
    // An aggregate as another's value: the inner one takes its `:` first. At a literal's start,
    // `min (…) : {…}`: the atom read again as the aggregate's value.
    ("r(n) :- n = sum min x : { s(x) } : { t(n) }, m = max sum x : { s(x) } + 1 : { t(m) }.", "parses"),
    ("r(n) :- n = sum min (x) : { s(x) } : { t(n) }, m = min min x by k : { s(x, k) } : { t(m) }.", "parses"),
    ("r(n) :- n = sum min min x : { a(x) } : { b(n) } : { c(n) }, k = sum join x sep \",\" : { s(x) } : { t(k) }.", "parses"),
    ("r(n) :- n = sum min(x) + 1 : { t(x) }, m = sum min() + 1 : { t(m) }, k = sum min - 1 : { t(k) }.", "parses"),
    ("r(n) :- min (x) : { s(x) } = n, max(y) : { s(y) } * 2 < n; mean (x + 1) : { s(x) } >= n, min(n).", "parses"),
    ("r(n) :- n = count : { mean (x) : { s(x) } > 1 }.", "parses"),
    // Fences of four backticks or more: pathological, so they go to the engine, which reads
    // them; inside a comment or a string they are text.
    ("r(x) :- x ~ ````a ``` b````, y ~ `````a````b`````, z ~ `c`.", "never"),
    ("r(x) :- x ~ ```` \"a // b ````, y = \"c\", z ~ ````a``` /* ``` */ ````.", "never"),
    ("r(x) :- x ~ ```````````a```````````.", "never"),
    ("r(x) :- x = 1. // ````\nr(y) :- y = \"````\". /* ````` */", "parses"),
    ("r(x) :- x ~ ````a`````, y = 1.", "never"),
    ("r(x) :- x ~ ```a```````b````.", "never"),
    ("r(x) :- x ~ ````a`````` `b`.", "never"),
    ("r(x) :- x ~ ````a``.", "never"),
    // `min()` and a `:`: an aggregate with no value, which the engine refuses, wherever it is.
    ("r(n) :- n = sum min() : { t(n) }.", "never"),
    ("r(n) :- n = sum min /* c */ ( ) // d\n : { t(n) }.", "never"),
    ("r(n) :- min() : { s(x) } = n.", "never"),
    ("r(n) :- min(a: x) : { s(x) } = n.", "never"),
    ("r(n) :- !min (x) : { s(x) } = n.", "never"),
    ("r(n) :- min (x) : { s(x) }.", "never"),
    ("r(-1).", "parses"),
    // Arithmetic: precedence, left to right; `-` and a number one leaf, across an extra too;
    // unary minus over a primary; parentheses; functor calls.
    ("r(x) :- s(y), x = y * 2 + -1, z = -(y - @f(x)) % 3.", "parses"),
    ("r(x) :- x = a - b - c, y = a + b * c - d / e % f, z = (a + b) * (c - d).", "parses"),
    ("r(- 1, - // c\n 2, -x, --1, -(1), - - x, -@f(1)).", "parses"),
    ("r(x) :- x + 1 > y, -x < 2, @f(x, -2) = 1, 1 + 2 ~ `a`, !x * 2 ~ `b`.", "parses"),
    ("r(k: v + 1, a * b: c) :- s(f(x) + g(y) * 2).", "parses"),
    // Disjunctions and groups; an atom or a call, by the token after it.
    ("r(x) :- s(x); t(x), u(x); v(x).", "parses"),
    ("r(x) :- s(x), (t(x); !(u(x), x > f(x) + 1)), !(v(x)).", "parses"),
    ("r(x) :- f(x) > 1, g(x) * 2 + 1 = h(y), k(x) ~ `a`, !m(x) ~ `b`, n(x).", "parses"),
    ("r(x) :- ((s(x); t(x)); (u(x))).", "parses"),
    // A parenthesized term at a literal's start: the engine tells it from a group by going
    // back after its `)`. The file goes to the engine.
    ("r(x) :- (x + 1) > 1.", "never"),
    // `.violation`: parts in any order, continuation lines, the words as names elsewhere.
    (".violation v(at: loc, c: symbol) code {c} in \"E[0-9]+\" tag {at} \"m {c}\"\nfix safe v_fix \"a fix\"\nsuppress ruff \"E501\", \"W291\" over {at}\noptions { \"a\": [1, {\"b\": 2}] }\nr(x) :- s(x).", "parses"),
    (".violation v(at: loc) code \"X1\" \"m\"\n.violation w(at: loc) \"n\" fix safe \"d\" fix f \"e\"\n.decl code(fix: number, tag: t)", "parses"),
    ("r(code, tag, fix, safe, suppress, over, options, in, violation) :- code(x), fix = 1.", "parses"),
    (".violation\nv(at: loc) tag { t } code { c } \"m\" // c\n/* d */ suppress t \"r\"", "parses"),
    // `fix`, `suppress`, `options` starting a clause right after a `.violation`, and `over` a
    // fact after a `suppress` line: the engine reads the word only before what follows it.
    (".violation v(at: loc) \"m\"\nfix(x) :- s(x).", "parses"),
    (".violation v(at: loc) \"m\"\nsuppress(x) :- s(x).\n.violation w(at: loc) \"m\"\noptions(x).", "parses"),
    (".violation v(at: loc) \"m\"\nsuppress t(x) :- s(x).", "never"),
    (".violation v(at: loc) \"m\"\nsuppress t \"r\"\nover(x).\nfix(1).", "parses"),
    (".violation v(at: loc) \"m\" fix /* c */ f // d\n \"e\" suppress t /* c */ \"r\" over /* c */ {at}\noptions // c\n{}\nfix(y).", "parses"),
    (".violation v(at: loc) \"m\"\nfix safe \"d\"\nfix safe s \"e\"\nfix \"x\".", "never"),
    (".violation v(at: loc) \"m\"\noptions = 1.", "never"),
    // A column in braces that is no single name: one `raw_text` leaf, nested braces counted.
    (".violation v(at: loc) code {at.code} tag { } \"m\"", "parses"),
    (".violation v(at: loc) code { /* c */ at } tag {a {b} c} \"m\" suppress t \"r\" over {x y}\n.violation w(at: loc) code {at} \"m\" suppress t \"r\" over { at }", "parses"),
    (".violation v(at: loc) code {at \"m\"", "never"),
    // A `` `` `` with none after it in the file's bytes: the empty literal, read on after it.
    ("r(x) :- x = replace(c, ``).", "parses"),
    ("r(x) :- y = ``a``, z = `b`, x = replace(c, ``). // c", "parses"),
    // One after it, in a comment or a string: the literal runs to it, as the engine reads it.
    ("r(x) :- x = ``. // `` later", "never"),
    ("r(x) :- x = ``, y = \"``\".", "never"),
    // JSON objects: terms, setting arguments; edits.
    ("r({\"a\": {\"b\": [1, 2]}}) :- s(x, {}), {\"c\": 1} = x => e(x, \"y\"), f({}).", "parses"),
    (".budget * by code { \"A\": 3, \"B\": 4 }\n.budget * {\n  \"total\": 0\n}\nr(x).", "parses"),
    ("r(n) :- n = sum {\"a\": 1} : { s(_) }, !{} ~ `x`.", "parses"),
    // A JSON object over lines ends its setting's line: the engine reads lines from its `{`.
    (".s {\n} x\nr(x).", "never"),
    // An atom before `=>`: the atom's level takes `=` (a call's comparison), not `=>`, which
    // its caller takes when the level ends.
    ("m(s) :- f(s) => r(a, `$sp`).\nm(s) :- f(s), g(s) => r(a) , q(b).", "parses"),
    // A comment after a setting's JSON object, before its line's end.
    (".budget * { \"total\": 0 }    // c\n.mode block", "parses"),
    // Policy documents: JSON, as the engine's reader takes it; else `.policy` is a setting.
    (".policy {\"format\": \"v1\", \"id\": \"p\", \"revision\": 2, \"rules\": [{\"a\": -1, \"b\": [true, false, null, 0, -0]}, []], \"e\": {}}\n.policy \"id\" revision 1\nr(x).", "parses"),
    (".policy {\n  \"a\": \"x\\\\y \\/ \\b\\f\\n\\r\\t\\\"\"\n} // c\n.\npolicy {} r(x).", "parses"),
    (".policy\n.policy x {\"a\": 1}\n.policy [1]", "parses"),
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
    // The other directives: all of `.rerail`.
    (".type t = /* c */ a\n.input r(IO=x /* d */, f=\"y\")\n.functor f(x: number): symbol stateful // e\n.edit e(a: node)\n.pragma \"a\" \"b\"\n.lattice l<> { A -> @f, B -> 1 + x }\n.input a, b(x)\n.output c", "parses"),
    // An empty raw text: no leaf, and its field on the node made last.
    (".type t\n.input r()\n.decl x(a: t)", "parses"),
    (".output\n.decl a(x: t)\n.input must(x), b\n.input r((a), b(c)), s(())", "parses"),
    (".type a = \"x\ny\"\nr(x).\n.type s <: number // c\n.pragma\n.pragma \"a\"", "parses"),
    // The rest of a `.type`'s line is the line its name ends on: a token after a string over
    // two lines starts the next statement.
    (".type a = \"x\ny\" z\nr(x).", "never"),
    (".lattice l<b> { }\n.lattice m<> { A -> @f(x) + 1, B -> -2 * x, C -> count : { s(_) } }", "parses"),
    (".functor f(x: number): t<u> stateful\n.functor g(): t<> x `y`\n.decl r(x: t) `lit` 1", "parses"),
    // A line's end met in a nested level that ends there (a return type): its caller's.
    (".functor dot(a: symbol): symbol    // c\n.functor e(a: symbol): t<u> // d\nr(x).", "parses"),
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
    ("a(x) :- b(o), (o - n) * 10 > o.\np(x) :- ((a(x))) + 1 > 2.\np(x) :- ((o)) ~ `p`, (b(x) ; c(x)), !(d(x)), ((a(x))).", "parses"),
    ("p(x) :- (o) > 1, (a(x)) = b, (count : { s(_) }) > 2, !(o) - 1 ~ `p`, ((o - n) > 1), ((a(x)) + 1 > 2).", "parses"),
    ("p(x) :- ( // c\n o - n) * 2 > 1, (a(x) /* d */) + 1 > 2, (-o) < 2, ((o)) * (n) > 1.", "parses"),
    ("p(x) :- count : { (o - 1) > 2 } = n, max x : { (s(x)) } > 1, !((a(x))).", "parses"),
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
    (".mode 1.5\nr(x) :- s(x, y), y > 0.10, x = -2.5 * y, z = - 3.25.", "parses"),
    (".scope 1.2.3\n.mode 1.2.3.4\nr(x) :- x = 1.2.", "parses"),
    ("r(x) :- x = 1.05, y = 007.5, z = 12345678901234567890.5, (a - 1.5) * 2.0 > b, c = -0.5.", "parses"),
    (".scope 1.2.3.4.5\n.mode 10.25.5 x\nr(x) :- x = 1.25.", "parses"),
    // The same, short of what goes to the engine (`TO_THE_ENGINE`).
    (".scope 1.2\n.mode 1.2\nr(x) :- x = 1.2.", "parses"),
    ("r(x) :- x = 12345678901234567890, y = 1.5.", "parses"),
    ("r(x) :- x = 1.05, y = 007.5, z = 123456789012345678, (a - 1.5) * 2.0 > b, c = -0.5.", "parses"),
    (".policy {\"a\": 1.5, \"b\": -0.25, \"c\": [1.0]}", "parses"),
    (".lattice l<b> { A -> 1.5 * x }\n.functor f(x: number): t 2.5\nr(x) :- count : { s(_) } > 1.5, max v : { t(v) } < -2.25.", "parses"),
    ("r(x) :- x = 1..5.", "never"),
    ("r(x) :- a = 1.2.3.4.", "never"),
    ("r(x) :- w = 1.5e3.", "never"),
    // `\u` in a string: the forms that are code points by their bytes.
    ("r(x) :- x = \"\\u212a\", y = \"\\u{1F600}\", z = \"\\u{41}\", w = \"a\\u017fb\\u0130\", v = \"\\u{10FFFF}\", u = \"\\u{FFFFF}\", t = \"\\\\u1234\".", "parses"),
    (".policy {\"a\": \"\\u00e9\", \"b\": \"x\"}", "parses"),
    ("r(x) :- x = \"\\uD800\".", "never"),
    ("r(x) :- x = \"\\u{D800}\", y = \"\\u{110000}\".", "never"),
    (".policy {\"a\": \"\\u{41}\"}", "never"),
    // The same bytes outside JSON: no refusal.
    ("r(x) :- x = a -b, s(\"a\\qb\", \"two\nlines\", 007) // c\n.", "parses"),
    // A rule file's tests (decision 0130): a fixture set, a test that uses it, with `exactly`, its
    // statements a file's; `.end` past a comment; `exactly` and `uses` as names.
    (".fixtures app\n  .fixture \"a.py\" ```\n    x = 1\n    ```\n.end\n.test t exactly uses app, b\n  #include \"c.rerail\"\n  seen(1).\n  r(x) :- seen(x).\n  .expect B101 at \"a.py\":1\n.end\nr(x) :- s(x).", "parses"),
    (".test exactly\n.end\n.test uses uses a\n. // c\n end\n.fixtures f exactly(1).\n.end", "parses"),
    // Blocks do not nest; a `.end` outside a block; a block with no `.end`.
    (".test a\n.test b\n.end\n.end", "never"),
    (".fixtures a\n.fixtures b\n.end\n.end", "never"),
    ("r(x).\n.end", "never"),
    (".test a\nr(x).", "never"),
    // `.input` with no relation, before an `#include` (`io_bare`'s `hash_word`, a keyword's
    // token), a comment between: the engine refuses it, so every step does.
    (".input #include \"a\"", "never"),
    (". /* a\n b */input // c\n#include \"a\"", "never"),
];

/// A node by names: kind, field ("" for none), start, end, size.
type Named = (String, String, usize, usize, usize);

/// The engine's tree of `src`, by names, and whether it holds an `ERROR`.
fn engine(src: &str) -> (Vec<Named>, bool) {
    let tape = rr::engine_tape(src);
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

/// The grammar's nodes by names: its ids back to the names they were made from.
fn named(c: &Compiled, nodes: &[IdNode]) -> Vec<Named> {
    let (kinds, fields) = c.ids.names();
    nodes
        .iter()
        .map(|n| {
            let field = if n.1 == 0 { String::new() } else { fields[&n.1].clone() };
            (kinds[&n.0].clone(), field, n.3, n.4, n.5)
        })
        .collect()
}

/// The engine's tree against the grammar's, for a document the grammar parses.
fn against_the_engine(_: &str, c: &Compiled, bytes: &[u8], want: &Option<Vec<IdNode>>) -> Option<String> {
    let nodes = want.as_ref()?;
    // A `.rerail` file is text: the engine reads only UTF-8.
    let src = std::str::from_utf8(bytes).ok()?;
    let (theirs, error) = engine(src);
    let ours = named(c, nodes);
    if error {
        return Some(format!("the grammar parses what the engine does not:\n  engine {theirs:?}\n  grammar {ours:?}"));
    }
    if ours == theirs {
        return None;
    }
    let at = ours.iter().zip(&theirs).position(|(a, b)| a != b).unwrap_or(ours.len().min(theirs.len()));
    Some(format!(
        "the trees differ at node {at}: grammar {:?}, engine {:?}\n  engine {theirs:?}\n  grammar {ours:?}",
        ours.get(at),
        theirs.get(at)
    ))
}

/// The grammar: the checks of `ladder`, and the engine's tree on every document it parses.
#[test]
fn rr_parses_as_the_reference_and_the_engine() {
    let grammar = grammar();
    let fixed: Vec<&str> = CASES.iter().map(|c| c.0).collect();
    let differ = check_ladder(&grammar, ALPHABET, SAMPLES, &fixed, &|name, c, bytes, want| against_the_engine(name, c, bytes, want));
    assert!(differ.is_empty(), "{} documents differ:\n{}", differ.len(), differ.join("\n"));
}

/// Cases the grammar sends to the engine, and the escape that sends it: an integer of 19 digits or more, its leading
/// zeros aside (the engine checks its i64). A float's chain of dots (`1.2.3`) and a float's digits, however many, are
/// read in tier 1, as a lexer reads them (decision 0400, Q40). The engine's tape is the file's either way.
const TO_THE_ENGINE: &[(&str, &str)] = &[("r(x) :- x = 12345678901234567890, y = 1.5.", "escape: a number of 19 digits or more")];

/// Each case marked `parses` parses, and the engine's tree has no `ERROR`; a case sent to the engine
/// (`TO_THE_ENGINE`) is refused by its escape alone.
#[test]
fn rr_cases_parse() {
    let mut missing = Vec::new();
    for (name, source) in grammar() {
        let c = rerail_cpu_stream::ladder::compile_step(&name, &source);
        for &(doc, mark) in CASES {
            let engine = TO_THE_ENGINE.iter().find(|e| e.0 == doc);
            let parsed = rerail_cpu_stream::ladder::reference(&c, doc.as_bytes()).is_some();
            match engine {
                Some(&(_, why)) => {
                    let refusal = rerail_cpu_stream::ladder::refusal(&c, doc.as_bytes());
                    if parsed || !refusal.starts_with(why) {
                        missing.push(format!("{name}: {doc:?}: not sent to the engine by {why:?}: {}", if parsed { "parsed" } else { &refusal }));
                    }
                }
                None if mark == "parses" && !parsed => missing.push(format!("{name}: {doc:?}: {}", rerail_cpu_stream::ladder::refusal(&c, doc.as_bytes()))),
                None => {}
            }
        }
    }
    assert!(TO_THE_ENGINE.iter().all(|e| CASES.iter().any(|c| c.0 == e.0)), "a case sent to the engine that is no case");
    assert!(missing.is_empty(), "cases the grammar refuses:\n{}", missing.join("\n"));
}

/// `refuse M in L`: the compiler takes it only where a refused mark is refused between the
/// level's first token and its last, as the reference refuses it: a hidden mark, nested levels,
/// each level they call refusing it too, no level that may end and go on.
#[test]
fn a_refused_mark_is_checked() {
    let source = grammar().remove(0).1;
    assert_eq!(rerail_cpu_stream::ladder::compile_error(&source), None);
    let refuse = "refuse json_bad in json_document json_inner_object json_inner_array";
    for (from, to, why) in [
        (refuse, "refuse json_bad in json_document json_inner_object", "calls one that does not"),
        (refuse, "refuse json_bad in json_document json_member json_inner_object json_inner_array", "no nested level"),
        (refuse, "refuse json_bad in source_file json_document json_inner_object json_inner_array", "the root"),
        ("hidden float_after line newline policy_on json_bad fix_on", "hidden float_after line newline policy_on fix_on", "no hidden mark"),
        ("merge json_object ']'", "merge json_object ']' [merge json_object '.']", "may end only after its last token"),
    ] {
        assert_eq!(source.matches(from).count(), 1, "{from}");
        let e = rerail_cpu_stream::ladder::compile_error(&source.replace(from, to));
        assert!(e.as_deref().is_some_and(|e| e.contains(why)), "{to}: {e:?}");
    }
}

/// `reread R`: the compiler takes a reading only by a name declared once, and gives its rows to
/// the program (`RETAG m=3` names one; `provisional` fields the program checks at `ACCEPT`).
#[test]
fn a_reading_is_checked() {
    let source = grammar().remove(0).1;
    assert_eq!(rerail_cpu_stream::ladder::compile_error(&source), None);
    for (from, to, why) in [
        ("reread parenthesized_strict pattern_tail\n  provisional", "reread nowhere pattern_tail\n  provisional", "no reading of that name"),
        ("reading parenthesized_strict = ", "reading parenthesized = ", "two of that name"),
        ("conjunction gone, atom as call", "conjunction gone, conjunction gone, atom as call", "a kind twice"),
    ] {
        assert_eq!(source.matches(from).count(), 1, "{from}");
        let e = rerail_cpu_stream::ladder::compile_error(&source.replace(from, to));
        assert!(e.as_deref().is_some_and(|e| e.contains(why)), "{to}: {e:?}");
    }
    let c = rerail_cpu_stream::ladder::compile_step("rr", &source);
    assert_eq!(c.program.readings.maps.len(), source.lines().filter(|l| l.trim_start().starts_with("reading ")).count());
    assert_eq!(c.program.readings.provisional, [c.ids.of("field bare")]);
    let rereads = c.native.structural.iter().filter(|b| matches!(b.operation, rerail_compiler::native_ir::StructuralOperation::Reread { .. })).count();
    assert!(rereads > 0, "the words reread");
}

/// A document's tokens and parse, for a look by hand: `RERAIL_RR_DOC` (the document, with `\n` for a line break).
#[test]
#[ignore = "a look by hand: run by name with RERAIL_RR_DOC"]
fn rr_look() {
    let Ok(doc) = std::env::var("RERAIL_RR_DOC") else { return };
    // `@path`: the document is that file.
    let doc = match doc.strip_prefix('@') {
        Some(path) => std::fs::read_to_string(path).expect("reads the file"),
        None => doc.replace("\\n", "\n"),
    };
    for (name, source) in grammar() {
        let c = rerail_cpu_stream::ladder::compile_step(&name, &source);
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
        eprintln!("  reference: {:?}", rerail_cpu_stream::ladder::refusal(&c, doc.as_bytes()));
        let compiled = rerail_cpu_stream::ladder::compiled_nodes(&c, doc.as_bytes());
        eprintln!("  compiled: {:?}", compiled.as_ref().map(|n| n.len()));
        // Where the compiled tree leaves the reference's: the nodes around it.
        if let (Ok(nodes), Some(want)) = (&compiled, rerail_cpu_stream::ladder::reference(&c, doc.as_bytes())) {
            let (ours, theirs) = (named(&c, &rerail_cpu_stream::ladder::as_ids(nodes.clone())), named(&c, &want));
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
        if let Some(nodes) = rerail_cpu_stream::ladder::reference(&c, doc.as_bytes()) {
            let ours = named(&c, &nodes);
            eprintln!("  reference tree: {ours:?}");
            let compiled = compiled.as_ref().ok().map(|n| named(&c, &rerail_cpu_stream::ladder::as_ids(n.clone())));
            eprintln!("  the engine's tree: {}; the compiled tree the reference's: {}", ours == theirs, compiled.as_ref() == Some(&ours));
            if let Some(at) = ours.iter().zip(&theirs).position(|(a, b)| a != b) {
                eprintln!("  the reference leaves the engine at node {at}: {:?} / {:?}", ours.get(at), theirs.get(at));
            }
        }
        eprintln!("  engine: {theirs:?}");
    }
}

/// Every `.rerail` file of the repository through the grammar's compiled path, against the engine's tree: the files it parses as the engine does,
/// those it parses differently or the engine refuses (each a bug), and those it sends to the
/// engine, by the reference's reason. Run by name.
#[test]
#[ignore = "the corpus: run by name"]
fn rr_corpus() {
    let (name, source) = grammar().remove(0);
    let c = rerail_cpu_stream::ladder::compile_step(&name, &source);
    let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("../../..");
    let mut files = Vec::new();
    let mut work = vec![root];
    while let Some(dir) = work.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let skip = p.file_name().is_some_and(|n| [".git", "target", "node_modules"].contains(&n.to_string_lossy().as_ref()));
            if p.is_dir() && !skip {
                work.push(p);
            } else if p.extension().is_some_and(|x| x == "rerail") {
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
        match rerail_cpu_stream::ladder::compiled_nodes(&c, &bytes) {
            Ok(nodes) => {
                let ours = named(&c, &rerail_cpu_stream::ladder::as_ids(nodes));
                let (theirs, error) = engine(src);
                if error {
                    refused_by_engine.push(f.display().to_string());
                } else if ours == theirs {
                    same += 1;
                    bytes_parsed += bytes.len();
                } else {
                    let at = ours.iter().zip(&theirs).position(|(a, b)| a != b).unwrap_or(ours.len().min(theirs.len()));
                    differ.push(format!("{}: node {at}: grammar {:?}, engine {:?}", f.display(), ours.get(at), theirs.get(at)));
                }
            }
            Err(_) => {
                let why = rerail_cpu_stream::ladder::refusal(&c, &bytes);
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
