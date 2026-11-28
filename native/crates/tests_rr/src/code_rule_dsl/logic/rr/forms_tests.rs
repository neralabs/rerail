//! Rust forms (`.form`, `preparation::forms`): a relation made by its registered Rust form gives its readers the rows
//! its rules give, made for the values its demand asks where every reader binds its key, else whole.

use std::path::Path;

/// `same(n, c)`, `points-to-file.rerail`'s `pt_tm_same`: how many of `n`'s ancestors in an unbroken chain span what
/// it does. `asked` binds its node (its demand keys it); `counts` binds nothing (it is made whole).
const RULES: &str = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, n: node, i: number)
.input node, child
.decl same(n: node, c: number)
.pragma "magic-transform" "same"
same(n, 0) :- node(_, n, _, _, _), !child(_, n, _).
same(n, 0) :- child(p, n, _), node(_, p, _, pb, _), node(_, n, _, b, _), pb != b.
same(n, 0) :- child(p, n, _), node(_, p, _, _, pe), node(_, n, _, _, e), pe != e.
same(n, c + 1) :- child(p, n, _), node(_, p, _, b, e), node(_, n, _, b, e), same(p, c).
"#;

const ASKED: &str = r#"
.decl asked(k: symbol, s: number, e: number, c: number)
asked(k, s, e, c) :- node(_, n, k, s, e), k = "call", same(n, c).
.output asked
"#;

const COUNTS: &str = r#"
.decl counts(c: number)
counts(c) :- same(_, c).
.output counts
"#;

const FORM: &str = ".form same tape.same_span\n";

/// A call under an expression statement and a parenthesized one; a decorated definition over its definition.
const A: &str = "def f(x):\n    return (g(x))\nf(1)\nprint((f(2)))\n@dec\ndef h():\n    pass\n";
const B: &str = "x = [k(i) for i in y]\n(z)\nw = (\n    # why\n    ((a and b))\n)\n";

fn corpus(tag: &str) -> std::path::PathBuf {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension(tag);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, text) in [("a.py", A), ("b.py", B)] {
        std::fs::write(dir.join(name), text).unwrap();
    }
    dir
}

fn rows(program: &str, dir: &Path, output: &str) -> Vec<Vec<String>> {
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(program, dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap_or_else(|e| panic!("{e}"));
    crate::code_rule_dsl::logic::plan::output::shown(&out, output).unwrap()
}

/// The profile's decisions on `topic` in `dir`.
fn decided(dir: &Path, topic: &str) -> Vec<String> {
    let text = std::fs::read_to_string(dir.join("profile.txt")).unwrap_or_default();
    let mut out = Vec::new();
    let mut on = false;
    for line in text.lines() {
        match line.strip_prefix("  - ") {
            Some(text) if on => out.push(text.to_owned()),
            Some(_) => {}
            None => on = line.starts_with(topic),
        }
    }
    out
}

/// Read with its key bound, a form is made for the keys its demand asks, and its readers' rows are its rules'.
#[test]
fn a_form_keyed_by_its_demand_gives_its_rules_rows() {
    let dir = corpus("form-asked");
    let prof = dir.join("prof");
    crate::code_rule_dsl::logic::profile::set(true);
    crate::code_rule_dsl::logic::profile::set_directory(&prof);
    let formed = rows(&format!("{RULES}{FORM}{ASKED}"), &dir, "asked");
    let made = decided(&prof, "Rust form result");
    let ruled = rows(&format!("{RULES}{ASKED}"), &corpus("form-asked-rules"), "asked");
    assert_eq!(formed, ruled);
    assert!(formed.iter().any(|r| r[3] != "0"), "a call spanned by its parent: {formed:?}");
    assert!(made.iter().any(|l| l.starts_with("same: form tape.same_span") && !l.contains("all keys")), "{made:?}");
}

/// Read with nothing bound, a form is made whole, and its readers' rows are its rules'.
#[test]
fn a_form_read_unbound_is_made_whole() {
    let dir = corpus("form-whole");
    let prof = dir.join("prof");
    crate::code_rule_dsl::logic::profile::set(true);
    crate::code_rule_dsl::logic::profile::set_directory(&prof);
    let formed = rows(&format!("{RULES}{FORM}{COUNTS}"), &dir, "counts");
    let made = decided(&prof, "Rust form result");
    let ruled = rows(&format!("{RULES}{COUNTS}"), &corpus("form-whole-rules"), "counts");
    assert_eq!(formed, ruled);
    assert!(formed.len() > 1, "{formed:?}");
    assert!(made.iter().any(|l| l.contains("all keys")), "{made:?}");
}

/// `down(m)`: the nodes reached down from a module through children whose parents' spans differ from theirs, as
/// `pt_scope_tm` is made down one scope at a time: the demand of `same` is made from `down`'s rows, which read it.
const RECURSIVE: &str = r#"
.decl down(n: node)
down(n) :- node(_, n, "module", _, _).
down(m) :- down(p), child(p, m, _), same(m, c), c < 1.
.decl reached(k: symbol, s: number, e: number)
reached(k, s, e) :- down(n), node(_, n, k, s, e).
.output reached
"#;

/// A form whose demand is made from what it makes is made whole, out of its readers' recursion, and gives its rules'
/// rows.
#[test]
fn a_form_asked_from_its_own_rows_is_made_whole() {
    let dir = corpus("form-recursive");
    let prof = dir.join("prof");
    crate::code_rule_dsl::logic::profile::set(true);
    crate::code_rule_dsl::logic::profile::set_directory(&prof);
    let formed = rows(&format!("{RULES}{FORM}{RECURSIVE}"), &dir, "reached");
    let made = decided(&prof, "Rust form result");
    let ruled = rows(&format!("{RULES}{RECURSIVE}"), &corpus("form-recursive-rules"), "reached");
    assert_eq!(formed, ruled);
    assert!(made.iter().any(|l| l.contains("all keys")), "{made:?}");
}

/// A form whose columns are not its relation's is refused, naming both.
#[test]
fn a_form_of_other_columns_is_refused() {
    let program = format!("{}{FORM}{ASKED}", RULES.replace("same(n: node, c: number)", "same(n: node, c: symbol)").replace("c + 1", "c"));
    let dir = corpus("form-refused");
    let out = dir.join("out");
    let err = crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).err().expect("refused");
    assert!(err.to_string().contains("tape.same_span makes columns"), "{err}");
}

/// `unparen(e, m)`, python/syntax's: `m` is `e` with its parentheses off, a comment inside them skipped. `inner`
/// binds `e` (its demand keys the form by `e`), `outer` binds `m` (by `m`), `both` reads it each way.
const UNPAREN: &str = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, n: node, i: number)
.decl kind(n: node, k: symbol)
.input node, child, kind
.decl unparen(e: node, m: node)
.pragma "magic-transform" "unparen"
unparen(e, e) :- kind(e, k), k != "parenthesized_expression".
unparen(e, m) :- kind(e, "parenthesized_expression"), child(e, i, _), !kind(i, "comment"), unparen(i, m).
"#;

const INNER: &str = r#"
.decl inner(s: number, k: symbol)
inner(s, k) :- node(_, a, "assignment", _, _), child(a, e, _), unparen(e, m), node(_, e, _, s, _), kind(m, k).
.output inner
"#;

const OUTER: &str = r#"
.decl outer(s: number, k: symbol)
outer(s, k) :- node(_, m, "boolean_operator", _, _), unparen(e, m), node(_, e, k, s, _).
.output outer
"#;

const UNWRAP: &str = ".form unparen tape.unwrap args parenthesized_expression comment\n";

/// `unparen`'s form gives its rules' rows read by `e`, by `m`, each way at once, and unbound.
#[test]
fn unparens_form_gives_its_rules_rows_by_either_column() {
    let unbound = ".decl pairs(s: number, t: number)\npairs(s, t) :- unparen(e, m), node(_, e, _, s, _), node(_, m, _, t, _).\n.output pairs\n";
    for (tag, readers, output) in [("inner", INNER.to_owned(), "inner"), ("outer", OUTER.to_owned(), "outer"), ("both", format!("{INNER}{OUTER}"), "outer"), ("unbound", unbound.to_owned(), "pairs")] {
        let dir = corpus(&format!("unwrap-{tag}"));
        let prof = dir.join("prof");
        crate::code_rule_dsl::logic::profile::set(true);
        crate::code_rule_dsl::logic::profile::set_directory(&prof);
        let formed = rows(&format!("{UNPAREN}{UNWRAP}{readers}"), &dir, output);
        let made = decided(&prof, "Rust form result");
        let ruled = rows(&format!("{UNPAREN}{readers}"), &corpus(&format!("unwrap-{tag}-rules")), output);
        assert_eq!(formed, ruled, "{tag}");
        assert!(!formed.is_empty(), "{tag}");
        assert!(made.iter().any(|l| l.starts_with("unparen: form tape.unwrap")), "{tag}: {made:?}");
    }
}

/// `name(s, x)`, python/points-to-file's `pt_tgt_name`: each identifier of `tgt` (here every identifier in a call's
/// arguments), its text and its nearest scope.
const NAMES: &str = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, n: node, i: number)
.decl kind(n: node, k: symbol)
.decl text(n: node, s: symbol)
.input node, child, kind, text
.decl scope(k: symbol)
scope("module"). scope("function_definition"). scope("class_definition"). scope("lambda").
.decl tgt(t: node)
tgt(t) :- kind(a, "argument_list"), child(a, t, _).
.decl name(s: node, x: symbol)
name(s, x) :- tgt(t), kind(t, "identifier"), text(t, x), nearest(t, s, scope).
.decl named(s: number, k: symbol, x: symbol)
named(b, k, x) :- name(s, x), node(_, s, k, b, _).
.output named
"#;

/// `pt_tgt_name`'s form gives its rules' rows: a name, its scope, for the nodes of the relation it reads.
#[test]
fn a_nearest_scopes_form_gives_its_rules_rows() {
    let form = ".form name tape.nearest_text reads tgt args identifier module function_definition class_definition lambda\n";
    let dir = corpus("nearest-text");
    let formed = rows(&format!("{NAMES}{form}"), &dir, "named");
    let ruled = rows(NAMES, &corpus("nearest-text-rules"), "named");
    assert_eq!(formed, ruled);
    assert!(formed.iter().any(|r| r[1] == "function_definition"), "{formed:?}");
}

/// `place(n, t)`, python/points-to-file's `pt_tm_place`: a node's term by its place under its nearest scope, here
/// with each scope named by its start.
const PLACES: &str = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, n: node, i: number)
.decl kind(n: node, k: symbol)
.decl host_node(n: node, h: node)
.input node, child, kind, host_node
.decl def(k: symbol)
def("function_definition"). def("class_definition").
.decl scope(k: symbol)
scope("module"). scope("function_definition"). scope("class_definition").
.decl scope_tm(s: node, q: symbol)
scope_tm(s, cat("s", to_string(b))) :- node(_, s, k, b, _), scope(k).
.decl same(n: node, c: number)
same(n, 0) :- node(_, n, _, _, _), !child(_, n, _).
same(n, 0) :- child(p, n, _), node(_, p, _, pb, _), node(_, n, _, b, _), pb != b.
same(n, 0) :- child(p, n, _), node(_, p, _, _, pe), node(_, n, _, _, e), pe != e.
same(n, c + 1) :- child(p, n, _), node(_, p, _, b, e), node(_, n, _, b, e), same(p, c).
.decl place(n: node, t: symbol)
place(n, cat(q, "@", to_string(b - sb), "+", to_string(e - b), "^", to_string(c))) :- node(_, n, k, b, e), !def(k),
    k != "module", !host_node(n, _), nearest(n, s, scope), node(_, s, _, sb, _), scope_tm(s, q), same(n, c).
.decl placed(b: number, k: symbol, t: symbol)
placed(b, k, t) :- place(n, t), node(_, n, k, b, _).
.output placed
"#;

/// `pt_tm_place`'s form gives its rule's rows, its scopes' terms read from the relation it reads.
#[test]
fn a_place_terms_form_gives_its_rules_rows() {
    let form = ".form place python.place_term reads scope_tm host_node args function_definition class_definition module / module function_definition class_definition\n";
    let formed = rows(&format!("{PLACES}{form}"), &corpus("place-term"), "placed");
    let ruled = rows(PLACES, &corpus("place-term-rules"), "placed");
    assert_eq!(formed, ruled);
    assert!(formed.iter().any(|r| r[2].starts_with("s") && !r[2].starts_with("s0@")), "{formed:?}");
    assert!(formed.iter().any(|r| r[2].ends_with("^1")), "{formed:?}");
}

/// `lit(s)`, python/annotation's `literal_str`: the nodes inside `Literal[...]`, `t.Literal[...]` and a generic
/// type's `Literal[...]`. Over core.rerail's relations, as the library reads them: `text` is any node's.
const LITERALS: &str = r#"#include "core.rerail"
.decl lit(s: node)
lit(s) :- kind(l, "subscript"), field(l, "value", v), (kind(v, "identifier") ; kind(v, "attribute")),
    text(v, t), (t = "Literal" ; suffix(".Literal", t)),
    within(s, l), !within(s, v).
lit(s) :- kind(l, "generic_type"), child(l, v, 0), text(v, "Literal"), within(s, l).
.decl lits(b: number, k: symbol)
lits(b, k) :- lit(s), node(_, s, k, b, _).
.output lits
"#;

/// `literal_str`'s form gives its rules' rows.
#[test]
fn a_literals_form_gives_its_rules_rows() {
    let text = "from typing import Literal\nimport typing\nx: Literal['a', \"b\"] = 'a'\ny: typing.Literal['c'] = 'c'\nz: dict[str, 'int'] = {}\ndef f(m: Literal['d']) -> Literal['e']: pass\n";
    let run = |program: &str, tag: &str| {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension(tag);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("c.py"), text).unwrap();
        rows(program, &dir, "lits")
    };
    let formed = run(&format!("{LITERALS}.form lit python.in_literal\n"), "in-literal");
    let ruled = run(LITERALS, "in-literal-rules");
    assert_eq!(formed, ruled);
    assert!(formed.iter().filter(|r| r[1] == "string").count() >= 5, "{formed:?}");
    // `typing.Literal['c']`: its value, the attribute, is in it, as `!within(s, v)` leaves it.
    assert!(formed.iter().any(|r| r[1] == "attribute"), "{formed:?}");
}

