//! Callable per-file relations in a corpus run (the "Callable Per-file Relations" page's spike, its corpus layer):
//! the cross part asks for a callable relation by its bound columns, and each ask is answered in the file it names,
//! over that file's relations held where its chunk made them, or where its memo entry kept them.

use std::path::Path;

/// Two functions that touch a name in common (`same_globals`, callable), compared only where one function calls both
/// (`runs_in`, cross-file). `touches` and `calls_in` are each file's.
const PROGRAM: &str = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl field(p: node, f: symbol, c: node)
.decl text(n: node, s: symbol)
.input node, field, text
.decl fname(d: number, f: node, s: symbol)
fname(d, f, s) :- node(d, f, "function_definition", _, _), field(f, "name", x), text(x, s).
.decl touches(d: number, g: symbol, v: symbol)
touches(d, g, v) :- fname(d, f, g), node(d, f, _, fs, fe), node(d, i, "identifier", s, e), fs < s, e <= fe, text(i, v).
.decl calls_in(d: number, b: symbol, g: symbol)
calls_in(d, b, g) :- fname(d, f, b), node(d, f, _, fs, fe), node(d, c, "call", s, e), fs < s, e <= fe, field(c, "function", x), text(x, g).
.decl runs_in(b: symbol, d: number, g: symbol) cross_file   // calls made together: a corpus's question (decision 0652)
runs_in(b, d, g) :- calls_in(d, b, g).
.decl same_globals(d: number, f: symbol, g: symbol, v: symbol)
.callable same_globals bbbf
same_globals(d, f, g, v) :- touches(d, f, v), touches(d, g, v), f != g.
.decl unsafe_pair(b: symbol, f: symbol, g: symbol, v: symbol)
unsafe_pair(b, f, g, v) :- runs_in(b, d, f), runs_in(b, d, g), same_globals(d, f, g, v).
.output unsafe_pair
"#;

const A: &str = "x = 0\ndef f0():\n    return x\ndef f2():\n    return x + 1\ndef f4():\n    return 2\ndef run():\n    f0()\n    f2()\n    f4()\n";
const B: &str = "y = 0\ndef f1():\n    return y\ndef f3():\n    return 3\ndef go():\n    f1()\n    f3()\n    f0()\n";
const C: &str = "def lone():\n    return x\ndef h():\n    return x\n";

fn corpus(tag: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension(tag);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, text) in files {
        std::fs::write(dir.join(name), text).unwrap();
    }
    dir
}

fn findings(program: &str, dir: &Path) -> Vec<Vec<String>> {
    let out = dir.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(program, dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap_or_else(|e| panic!("{e}"));
    crate::code_rule_dsl::logic::plan::output::shown(&out, "unsafe_pair").unwrap()
}

/// The program with `same_globals` made whole, as it was before it was callable.
fn whole() -> String {
    PROGRAM.replace(".callable same_globals bbbf\n", "")
}

/// A callable relation is answered in its readers' join: the corpus run's findings are those of the relation made
/// whole.
#[test]
fn a_callable_relation_is_answered_in_its_readers_join() {
    let files = [("a.py", A), ("b.py", B), ("c.py", C)];
    let called = findings(PROGRAM, &corpus("callable-called", &files));
    let made = findings(&whole(), &corpus("callable-whole", &files));
    assert_eq!(called, made);
    assert_eq!(called, [["run", "f0", "f2", "x"], ["run", "f2", "f0", "x"]]);
}

/// A call names one file, its column 0, and reads that file's relations alone: `go` in b.py calls `f0`, defined in
/// a.py, so the asks about it are answered from b.py, which defines no `f0`, and give nothing.
#[test]
fn a_call_across_two_files_definitions_enters_neither() {
    let files = [("a.py", A), ("b.py", B)];
    let called = findings(PROGRAM, &corpus("callable-across", &files));
    assert!(called.iter().all(|r| r[0] != "go"), "{called:?}");
    assert_eq!(called, findings(&whole(), &corpus("callable-across-whole", &files)));
}

/// An edit to a file invalidates what called into it: the run after the edit (its other files from the memo, their
/// held rows from their entries) equals a fresh run of the edited corpus.
#[test]
fn an_edit_invalidates_what_called_into_the_file() {
    let dir = corpus("callable-edit", &[("a.py", A), ("b.py", B), ("c.py", C)]);
    assert_eq!(findings(PROGRAM, &dir).len(), 2);
    let edited = A.replace("return 2", "return x * 2");
    std::fs::write(dir.join("a.py"), &edited).unwrap();
    let again = findings(PROGRAM, &dir);
    let fresh = findings(PROGRAM, &corpus("callable-edit-fresh", &[("a.py", &edited), ("b.py", B), ("c.py", C)]));
    assert_eq!(again, fresh);
    assert_eq!(again.len(), 6, "f0, f2 and f4 each with the others, over x: {again:?}");
    // And an edit to a file nothing calls into (c.py) changes nothing, its asks none.
    std::fs::write(dir.join("c.py"), "def lone():\n    return 1\n").unwrap();
    assert_eq!(findings(PROGRAM, &dir), fresh);
}
