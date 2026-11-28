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

/// `place(n, t)`, python/points-to-file's `pt_tm`: a scope's term, a guest's (here every integer's), and every other
/// node's by its place under its nearest scope, each scope named by its start.
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
.decl guest(n: node, t: symbol)
guest(n, "g") :- kind(n, "integer").
.decl place(n: node, t: symbol)
place(s, q) :- scope_tm(s, q).
place(n, t) :- guest(n, t).
place(n, cat(q, "@", to_string(b - sb), "+", to_string(e - b), "^", to_string(c))) :- node(_, n, k, b, e), !def(k),
    k != "module", !host_node(n, _), nearest(n, s, scope), node(_, s, _, sb, _), scope_tm(s, q), same(n, c).
.decl placed(b: number, k: symbol, t: symbol)
placed(b, k, t) :- place(n, t), node(_, n, k, b, _).
.output placed
"#;

/// `pt_tm`'s form gives its rules' rows, its scopes' and guests' terms read from the relations it reads.
#[test]
fn a_place_terms_form_gives_its_rules_rows() {
    let form = ".form place python.term reads scope_tm host_node guest args function_definition class_definition module / module function_definition class_definition\n";
    let formed = rows(&format!("{PLACES}{form}"), &corpus("place-term"), "placed");
    let ruled = rows(PLACES, &corpus("place-term-rules"), "placed");
    assert_eq!(formed, ruled);
    assert!(formed.iter().any(|r| r[2].starts_with("s") && !r[2].starts_with("s0@")), "{formed:?}");
    assert!(formed.iter().any(|r| r[2].ends_with("^1")), "{formed:?}");
    assert!(formed.iter().any(|r| r[2] == "g") && formed.iter().any(|r| r[1] == "module" && r[2] == "s0"), "{formed:?}");
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


/// `ctx(s)`, python/annotation's `ann_ctx`: the nodes of the annotations' roots' subtrees, a root under another's
/// among them (a default parameter's annotation inside a lambda's, none here; a nested subscript's).
const SUBTREES: &str = r#"#include "core.rerail"
.decl root(t: node)
root(t) :- kind(p, k), (k = "typed_parameter" ; k = "typed_default_parameter"), field(p, "type", t).
root(t) :- kind(f, "function_definition"), field(f, "return_type", t).
root(t) :- kind(a, "assignment"), field(a, "type", t).
root(t) :- kind(t, "subscript").
.decl ctx(s: node)
ctx(s) :- root(a), (s = a ; within(s, a)).
.decl ctxs(b: number, k: symbol)
ctxs(b, k) :- ctx(s), node(_, s, k, b, _).
.output ctxs
"#;

/// `ann_ctx`'s form gives its rule's rows.
#[test]
fn a_subtrees_form_gives_its_rules_rows() {
    let text = "x: dict[str, list[int]] = {}\ndef f(m: 'A', n: int = 1) -> list[str]: return m[0]\n";
    let run = |program: &str, tag: &str| {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension(tag);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("c.py"), text).unwrap();
        rows(program, &dir, "ctxs")
    };
    let formed = run(&format!("{SUBTREES}.form ctx tape.subtrees reads root\n"), "subtrees");
    let ruled = run(SUBTREES, "subtrees-rules");
    assert_eq!(formed, ruled);
    assert!(formed.iter().any(|r| r[1] == "subscript") && formed.iter().any(|r| r[1] == "string"), "{formed:?}");
}

/// `start(n, s)`, python/scopes' `wstart`, over its rules' own inputs: a name's nearest scope or comprehension, a
/// parameter's default or annotation started outside its definition, a comprehension's first iterable outside it,
/// and (standing in for a string annotation's names) each name in a call `g(...)` started at the module.
const STARTS: &str = r#"#include "core.rerail"
.decl scope(kind: symbol)
scope("module"). scope("function_definition"). scope("class_definition"). scope("lambda").
.decl comp_scope(kind: symbol)
comp_scope("list_comprehension"). comp_scope("set_comprehension"). comp_scope("dictionary_comprehension").
comp_scope("generator_expression").
.decl lscope(kind: symbol)
lscope(k) :- scope(k).
lscope(k) :- comp_scope(k).
.decl nameref(n: node, x: symbol)
nameref(n, x) :- kind(n, "identifier"), text(n, x), !field(_, "attribute", n), !field(_, "name", n).
.decl first_for(c: node, fc: node)
first_for(c, fc) :- comp_scope(k), kind(c, k), child(c, fc, i), kind(fc, "for_in_clause"),
    count : { child(c, g, j), kind(g, "for_in_clause"), j < i } = 0.
.decl def_outside(n: node, f: node)
def_outside(n, f) :- nameref(n, _), nearest(n, f, lscope), (kind(f, "function_definition") ; kind(f, "lambda")),
    (field(f, "parameters", p) ; field(f, "return_type", p)), within(n, p).
.decl in_first_iter(n: node, c: node)
in_first_iter(n, c) :- nameref(n, _), nearest(n, c, lscope), comp_scope(k), kind(c, k), first_for(c, fc),
    field(fc, "right", e), (n = e ; within(n, e)).
.decl hosted(n: node, h: node)
hosted(n, c) :- kind(c, "call"), field(c, "function", g), text(g, "g"), within(n, c), kind(n, "identifier").
.decl starts_at(h: node, s: node)
starts_at(c, m) :- kind(c, "call"), field(c, "function", g), text(g, "g"), kind(m, "module"), docof(c, d), docof(m, d).
.decl start(n: node, s: node)
start(n, s) :- nameref(n, _), nearest(n, s, lscope), kind(s, k), scope(k), !def_outside(n, _).
start(n, c) :- nameref(n, _), nearest(n, c, lscope), kind(c, k), comp_scope(k), !in_first_iter(n, c).
start(n, s) :- def_outside(n, f), nearest(f, s, lscope).
start(n, s) :- in_first_iter(n, c), nearest(c, s, lscope).
start(n, s) :- nameref(n, _), hosted(n, h), starts_at(h, s).
.decl starts(b: number, x: symbol, sb: number, k: symbol)
starts(b, x, sb, k) :- start(n, s), node(_, n, _, b, _), text(n, x), node(_, s, k, sb, _).
.output starts
"#;

/// `wstart`'s form gives its rules' rows.
#[test]
fn a_walk_starts_form_gives_its_rules_rows() {
    let text = "class C:\n    y = [a for a in xs if a]\n    def m(self, k=d, *, j: T = lambda q=r: q) -> R:\n        return {v: w for v in self.ys for w in v}\nz = g(p, (h for h in hs))\n";
    let run = |program: &str, tag: &str| {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension(tag);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("c.py"), text).unwrap();
        rows(program, &dir, "starts")
    };
    let form = ".form start python.wstart reads nameref def_outside in_first_iter hosted starts_at args module function_definition class_definition lambda / list_comprehension set_comprehension dictionary_comprehension generator_expression\n";
    let formed = run(&format!("{STARTS}{form}"), "wstart");
    let ruled = run(STARTS, "wstart-rules");
    assert_eq!(formed, ruled);
    // `xs` (a first iterable) starts at the class, `d` and `T` (a default, an annotation) at the class too, `r` (a
    // lambda's default in a parameter's) at the method.
    let at = |x: &str| formed.iter().filter(|r| r[1] == x).map(|r| r[3].clone()).collect::<Vec<_>>();
    assert_eq!((at("xs"), at("d"), at("T"), at("r")), (vec!["class_definition".to_owned()], vec!["class_definition".to_owned()], vec!["class_definition".to_owned()], vec!["function_definition".to_owned()]), "{formed:?}");
}

/// `pos(n)`, python/points-to-file's `pt_vpos`: the positions whose value is read, down from roots (a call's arguments,
/// a return's value, a statement's assignment) through parentheses, conditionals, `and`/`or`, assignments' values and
/// walruses' values, and a subscript's value (`o[i]` reads `o`); a node within a call of `g` (standing in for a
/// guest's) is no root.
const REACH: &str = r#"#include "core.rerail"
.decl host(n: node, h: node)
host(n, c) :- kind(c, "call"), field(c, "function", g), text(g, "g"), (n = c ; within(n, c)).
.decl root(e: node)
root(a) :- kind(c, "call"), field(c, "arguments", l), child(l, a, _).
root(e) :- kind(r, "return_statement"), child(r, e, 0).
root(a) :- kind(s, "expression_statement"), child(s, a, 0), kind(a, "assignment").
.decl pos(n: node)
pos(e) :- root(e), !host(e, _).
pos(c) :- pos(n), kind(n, "parenthesized_expression"), child(n, c, 0).
pos(c) :- pos(n), kind(n, "conditional_expression"), child(n, c, 0).
pos(c) :- pos(n), kind(n, "conditional_expression"), child(n, c, 2).
pos(c) :- pos(n), kind(n, "boolean_operator"), child(n, c, _).
pos(c) :- pos(n), kind(n, "assignment"), field(n, "right", c).
pos(c) :- pos(n), kind(n, "named_expression"), field(n, "value", c).
pos(o) :- pos(c), kind(c, "subscript"), field(c, "value", o), !host(o, _).
.decl poss(b: number, k: symbol)
poss(b, k) :- pos(n), node(_, n, k, b, _).
.output poss
"#;

/// `pt_vpos`'s form gives its rules' rows.
#[test]
fn a_reach_form_gives_its_rules_rows() {
    let text = "f(a if b else (c or d), g(1)[0], g(2), h(k)[m][n])\nx = y = (z := w)\ndef h():\n    return (p and q)[1]\n";
    let run = |program: &str, tag: &str| {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension(tag);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("c.py"), text).unwrap();
        rows(program, &dir, "poss")
    };
    let form = ".form pos tape.reach reads root host args parenthesized_expression#0 conditional_expression#0 conditional_expression#2 boolean_operator#* assignment.right named_expression.value =subscript.value\n";
    let formed = run(&format!("{REACH}{form}"), "reach");
    let ruled = run(REACH, "reach-rules");
    assert_eq!(formed, ruled);
    // Of the calls, `h(k)` (at 39, a subscript's value) alone: `g(1)` (24) is a subscript's value within a call of
    // `g`, `g(2)` (33) one itself.
    let calls: Vec<&str> = formed.iter().filter(|r| r[1] == "call").map(|r| r[0].as_str()).collect();
    assert_eq!(calls, ["39"], "{formed:?}");
    assert_eq!(formed.iter().filter(|r| r[1] == "named_expression").count(), 1, "{formed:?}");
}

/// `cond(c)`, python/conditions' `py_cond`: an `if`'s or `elif`'s condition, and what a `not` of one tests, through
/// one pair of parentheses (`not (t)` tests `t`).
const CONDS: &str = r#"#include "core.rerail"
.decl cond(c: node)
cond(c) :- kind(i, "if_statement"), field(i, "condition", c).
cond(c) :- kind(i, "elif_clause"), field(i, "condition", c).
.decl neg(c: node, a: node)
neg(c, a) :- cond(c), kind(c, "not_operator"), field(c, "argument", a), !kind(a, "parenthesized_expression").
neg(c, a) :- cond(c), kind(c, "not_operator"), field(c, "argument", p), kind(p, "parenthesized_expression"), child(p, a, 0).
cond(a) :- neg(_, a).
.decl conds(b: number, k: symbol, x: symbol)
conds(b, k, x) :- cond(n), node(_, n, k, b, _), text(n, x).
.output conds
"#;

/// `py_cond`'s form, its roots found on the tape, gives its rules' rows.
#[test]
fn a_reach_form_from_kinds_gives_its_rules_rows() {
    let text = "if not (not x):\n    pass\nelif not y and z:\n    pass\nif a:\n    pass\nif not ((b)):\n    pass\n";
    let run = |program: &str, tag: &str| {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension(tag);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("c.py"), text).unwrap();
        rows(program, &dir, "conds")
    };
    let form = ".form cond tape.reach args ^if_statement.condition ^elif_clause.condition not_operator.argument~parenthesized_expression\n";
    let formed = run(&format!("{CONDS}{form}"), "conds");
    let ruled = run(CONDS, "conds-rules");
    assert_eq!(formed, ruled);
    let texts: Vec<&str> = formed.iter().map(|r| r[2].as_str()).collect();
    assert!(texts.contains(&"x") && texts.contains(&"not x") && texts.contains(&"a") && texts.contains(&"(b)"), "{formed:?}");
}

/// Classes, methods, a nested function, a lambda, a definition in a function, attributes and a match's patterns.
const NAMES_TEXT: &str = "import os\nclass C:\n    def m(self, k=1):\n        def inner(): pass\n        return os.path.join(k, f(x=1))\n    class D:\n        def n(self): pass\nlam = lambda: 0\ndef top():\n    class E: pass\nmatch v:\n    case Point(x=px): pass\n    case os.PathLike(): pass\n";

/// `program`'s `outputs` over `NAMES_TEXT`, in a directory of `tag`.
fn names_rows(program: &str, tag: &str, outputs: &[&str]) -> Vec<Vec<Vec<String>>> {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension(tag);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("c.py"), NAMES_TEXT).unwrap();
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap_or_else(|e| panic!("{e}"));
    outputs.iter().map(|o| crate::code_rule_dsl::logic::plan::output::shown(&out, o).unwrap()).collect()
}

/// python/scopes' `def_name` and `qualname`: a definition's name, and its dotted path from its module.
const QUALS: &str = r#"#include "core.rerail"
.decl scope(kind: symbol)
scope("module"). scope("function_definition"). scope("class_definition"). scope("lambda").
.decl modname(m: node, q: symbol)
modname(m, q) :- node(d, m, "module", _, _), doc(d, _, q).
.decl def_name(f: node, s: symbol)
def_name(f, s) :- kind(f, k), (k = "function_definition" ; k = "class_definition"), field(f, "name", n), text(n, s).
.decl qualname(f: node, q: symbol)
qualname(f, @dot(m, x)) :- def_name(f, x), nearest(f, s, scope), modname(s, m).
qualname(f, @dot(p, x)) :- def_name(f, x), nearest(f, s, scope), !kind(s, "module"), qualname(s, p).
.decl names(b: number, x: symbol)
names(b, x) :- def_name(f, x), node(_, f, _, b, _).
.decl quals(b: number, q: symbol)
quals(b, q) :- qualname(f, q), node(_, f, _, b, _).
.output names
.output quals
"#;

/// `def_name`'s and `qualname`'s forms give their rules' rows, the second read over either of the first.
#[test]
fn a_dotted_paths_form_gives_its_rules_rows() {
    let names = ".form def_name tape.field_text args function_definition class_definition / name\n";
    let quals = ".form qualname tape.dotted reads def_name modname args module function_definition class_definition lambda\n";
    let ruled = names_rows(QUALS, "quals-rules", &["names", "quals"]);
    assert_eq!(names_rows(&format!("{QUALS}{names}"), "quals-names", &["names", "quals"]), ruled);
    assert_eq!(names_rows(&format!("{QUALS}{quals}"), "quals-quals", &["names", "quals"]), ruled);
    assert_eq!(names_rows(&format!("{QUALS}{names}{quals}"), "quals-both", &["names", "quals"]), ruled);
    let quals: Vec<&str> = ruled[1].iter().map(|r| r[1].as_str()).collect();
    assert!(quals.contains(&"c.C.m.inner") && quals.contains(&"c.C.D.n") && quals.contains(&"c.top.E"), "{quals:?}");
}

/// python/scopes' `attr_of` and `nameref`: `o.a`'s object and name, and the identifiers that name (not an attribute's
/// name, a definition's or keyword's name, nor a keyword pattern's key).
const REFS: &str = r#"#include "core.rerail"
.decl attr_of(n: node, o: node, a: symbol)
attr_of(n, o, a) :- kind(n, "attribute"), field(n, "object", o), field(n, "attribute", at), text(at, a).
.decl kwpat_key(n: node)
kwpat_key(n) :- kind(k, "keyword_pattern"), child(k, n, 0).
.decl nameref(n: node, x: symbol)
nameref(n, x) :- kind(n, "identifier"), text(n, x), !field(_, "attribute", n), !field(_, "name", n), !kwpat_key(n).
.decl refs(b: number, x: symbol)
refs(b, x) :- nameref(n, x), node(_, n, _, b, _).
.decl attrs(b: number, ob: number, a: symbol)
attrs(b, ob, a) :- attr_of(n, o, a), node(_, n, _, b, _), node(_, o, _, ob, _).
.output refs
.output attrs
"#;

/// `attr_of`'s and `nameref`'s forms give their rules' rows.
#[test]
fn field_and_text_forms_give_their_rules_rows() {
    let forms = ".form attr_of tape.field_and_text args attribute / object attribute\n.form nameref tape.text args identifier / attribute name reads kwpat_key\n";
    let ruled = names_rows(REFS, "refs-rules", &["refs", "attrs"]);
    let formed = names_rows(&format!("{REFS}{forms}"), "refs", &["refs", "attrs"]);
    assert_eq!(formed, ruled);
    let refs: Vec<&str> = ruled[0].iter().map(|r| r[1].as_str()).collect();
    assert!(refs.contains(&"os") && refs.contains(&"px") && !refs.contains(&"join") && !refs.contains(&"inner") && !refs.contains(&"x"), "{refs:?}");
    assert_eq!(ruled[1].iter().map(|r| r[2].as_str()).collect::<Vec<_>>(), ["join", "path"], "{:?}", ruled[1]);
}
