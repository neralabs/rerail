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

/// The decisions of `topics` in the profile report a run wrote to `dir`: a topic's line, then a `- ` line for each
/// run that decided it (the program's, and its phases').
fn decided(dir: &Path, topics: &[&str]) -> Vec<String> {
    let text = std::fs::read_to_string(dir.join("profile.txt")).unwrap_or_default();
    let mut out = Vec::new();
    let mut topic: Option<&str> = None;
    for line in text.lines() {
        match line.strip_prefix("  - ") {
            Some(text) => {
                if let Some(t) = topic {
                    out.push(format!("{t} {text}"));
                }
            }
            None => topic = topics.iter().any(|t| line.starts_with(t)).then_some(line),
        }
    }
    out
}

/// `PROGRAM` keyed by name, not by file: `qsame` is asked by two functions' qualified names (`d:f`, their document's
/// number and name), and its asks are routed by `qhome`, which reads the file off the first (before `:`), as a
/// language's library reads a name's home off it. No column of `qsame` names a document (what it reads does: a file's
/// held rows are kept by it).
const ROUTED: &str = r#"
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
.decl runs_in(b: symbol, d: number, g: symbol) cross_file
runs_in(b, d, g) :- calls_in(d, b, g).
.decl qtouch(d: number, q: symbol, v: symbol)
qtouch(d, q, v) :- touches(d, f, v), q = cat(to_string(d), ":", f).
.decl qhome__ask(q: symbol)
.decl qhome(q: symbol, d: number) cross_file   // every file's path: a router reads them all
qhome(q, d) :- qhome__ask(q), match("(?<x>[0-9]+):.*", q), d = to_number(x).
.decl qsame(f: symbol, g: symbol, v: symbol)
.callable qsame bbf by qhome
qsame(f, g, v) :- qtouch(d, f, v), qtouch(d, g, v), f != g.
.decl unsafe_pair(b: symbol, f: symbol, g: symbol, v: symbol)
unsafe_pair(b, f, g, v) :- runs_in(b, d, f), runs_in(b, d, g), qf = cat(to_string(d), ":", f), qg = cat(to_string(d), ":", g), qsame(qf, qg, v).
.output unsafe_pair
"#;

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

/// A callable asked by name is answered in the file its router reads off the key (`by qhome`): the findings are those
/// of `qsame` made whole, and those of the program keyed by file.
#[test]
fn a_call_routed_by_its_key_is_answered_in_the_file_the_key_names() {
    let files = [("a.py", A), ("b.py", B), ("c.py", C)];
    let dir = corpus("callable-routed", &files);
    let prof = dir.join("prof");
    crate::code_rule_dsl::logic::profile::set(true);
    crate::code_rule_dsl::logic::profile::set_directory(&prof);
    let called = findings(ROUTED, &dir);
    // The asks reached the calls, each in the file its key names (a router made in each file routes none).
    let calls = decided(&prof, &["callable relations' calls"]);
    assert!(calls.iter().any(|l| !l.contains("batches, 0 requests") && l.contains("(0 requests of no file)")), "{calls:?}");
    let made = findings(&ROUTED.replace(".callable qsame bbf by qhome\n", ""), &corpus("callable-routed-whole", &files));
    assert_eq!(called, made);
    assert_eq!(called, [["run", "f0", "f2", "x"], ["run", "f2", "f0", "x"]]);
}

/// Demand (`.pragma "magic-transform"`) keeps a callable's router: no output reads it, only the request the callable
/// split writes after demand, so demand roots it as an output, free. Left out, it dropped the router's rules and the
/// request read an empty router: no call, and `qsame` empty.
#[test]
fn a_routed_call_under_demand_keeps_its_router() {
    let files = [("a.py", A), ("b.py", B), ("c.py", C)];
    let dir = corpus("callable-routed-demand", &files);
    let prof = dir.join("prof");
    crate::code_rule_dsl::logic::profile::set(true);
    crate::code_rule_dsl::logic::profile::set_directory(&prof);
    let program = format!("{ROUTED}\n.pragma \"magic-transform\" \"qtouch\"\n");
    let called = findings(&program, &dir);
    let calls = decided(&prof, &["callable relations' calls"]);
    assert!(calls.iter().any(|l| !l.contains("batches, 0 requests") && l.contains("(0 requests of no file)")), "{calls:?}");
    assert_eq!(called, [["run", "f0", "f2", "x"], ["run", "f2", "f0", "x"]]);
}

/// A router's table of the files is each file's relation, read by the cross part as any export (a language's module
/// table: `fdoc`, a document's number as text): the split keeps it in the files and exports it.
#[test]
fn a_routers_table_of_the_files_is_each_files_relation() {
    let program = ROUTED.replace(
        r#"qhome(q, d) :- qhome__ask(q), match("(?<x>[0-9]+):.*", q), d = to_number(x)."#,
        ".decl fdoc(d: number, x: symbol)\nfdoc(d, to_string(d)) :- node(d, _, \"module\", _, _).\nqhome(q, d) :- qhome__ask(q), match(\"(?<x>[0-9]+):.*\", q), fdoc(d, x).",
    );
    assert!(program.contains("fdoc(d, x)"), "{program}");
    let v = crate::code_rule_dsl::logic::rr::parse_version(&program).unwrap();
    // The request the callable split writes (`qsame__call`, read through `qhome`) is what the cone is made from.
    let v = crate::code_rule_dsl::logic::plan::corpus::preparation::callable::split(&v).unwrap().expect("a callable").caller;
    let pre = crate::code_rule_dsl::typing::file_local::programs_of(v).unwrap();
    assert!(pre.file_local.contains("fdoc"), "fdoc is cross-file: {:?}", pre.causes.get("fdoc"));
    // Read by the cone of requests, it is exported: a projection of `node` derived again in the cross part read the
    // stored input, whose rows no load answers ("the cone of requests reads base relation node").
    assert!(pre.exports.iter().any(|x| x == "fdoc"), "fdoc is not exported: {:?}", pre.exports);
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

/// The page's measurement: 3 files of `n` functions, each touching `x` (so every pair of a file shares it), and one
/// function a file calling 4 of them (12 ordered pairs asked a file). `mode` is `whole` (`same_globals` made in full:
/// n(n-1) rows a file), `magic` (made whole, demand by magic sets: `.pragma "magic-transform"`) or `called` (the 12
/// asked). One configuration a process (`cargo test … callable_measure_whole_100 -- --ignored`), so its peak is its
/// own: a line of rows, findings, wall and CPU seconds, and peak MB.
fn measure(n: usize, mode: &str) {
    let mut text = String::from("x = 0\n");
    for i in 0..n {
        text.push_str(&format!("def f{i}():\n    return x\n"));
    }
    text.push_str("def run():\n    f0()\n    f1()\n    f2()\n    f3()\n");
    let files: Vec<(String, String)> = ["a.py", "b.py", "c.py"].iter().map(|p| (p.to_string(), text.clone())).collect();
    let borrowed: Vec<(&str, &str)> = files.iter().map(|(p, t)| (p.as_str(), t.as_str())).collect();
    let dir = corpus(&format!("callable-measure-{mode}-{n}"), &borrowed);
    let program = match mode {
        "called" => PROGRAM.to_owned(),
        "whole" => whole(),
        "magic" => format!("{}.pragma \"magic-transform\" \"same_globals\"\n", whole()),
        m => panic!("{m}: not a mode"),
    };
    let out = dir.join("out");
    // Profiled in every mode (like with like): the calls' line is a decision of the run's report.
    let prof = dir.join("prof");
    crate::code_rule_dsl::logic::profile::set(true);
    crate::code_rule_dsl::logic::profile::set_directory(&prof);
    let (wall, cpu) = (std::time::Instant::now(), rerail_prof::process_cpu());
    let report = crate::code_rule_dsl::logic::plan::corpus::run(&program, &dir, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap_or_else(|e| panic!("{e}"));
    let (wall, cpu) = (wall.elapsed().as_secs_f64(), (rerail_prof::process_cpu() - cpu).as_secs_f64());
    let found = crate::code_rule_dsl::logic::plan::output::shown(&out, "unsafe_pair").unwrap();
    let _ = report;
    let peak = rerail_prof::code_rule_dsl::prof::max_footprint_mb();
    // The rows the files made of `same_globals` (handed on whole; none, called) and the calls' answers.
    let lines = decided(&prof, &["callable", "rows handed on to the cross-file part"]);
    eprintln!("callable_measure {mode:>6} n={n:>5}: unsafe_pair {}, wall {wall:.2} s, cpu {cpu:.2} s, peak {peak} MB; {lines:?}", found.len());
    // 12 ordered pairs of f0..f3, over x: the same in each file, and `unsafe_pair` names no file.
    assert_eq!(found.len(), 12);
    let _ = std::fs::remove_dir_all(&dir);
}

macro_rules! measures {
    ($($name:ident: $n:expr, $mode:expr;)*) => {$(
        #[test]
        #[ignore = "a measurement: one configuration a process"]
        fn $name() {
            measure($n, $mode);
        }
    )*};
}

measures! {
    callable_measure_whole_10: 10, "whole";
    callable_measure_magic_10: 10, "magic";
    callable_measure_called_10: 10, "called";
    callable_measure_whole_100: 100, "whole";
    callable_measure_magic_100: 100, "magic";
    callable_measure_called_100: 100, "called";
    callable_measure_whole_1000: 1000, "whole";
    callable_measure_magic_1000: 1000, "magic";
    callable_measure_called_1000: 1000, "called";
}

/// The run names the files' relations its cross part could call, by their rows: made whole, `same_globals` is one,
/// its pattern what `unsafe_pair` binds (`bbbf`).
#[test]
fn a_run_names_the_relations_it_could_call() {
    // A corpus of its own: the same files and program as another test's would replay that run's frame, deciding nothing.
    let dir = corpus("callable-candidates", &[("a.py", A), ("b.py", B), ("d.py", "def k():\n    return 1\n")]);
    let prof = dir.join("prof");
    crate::code_rule_dsl::logic::profile::set(true);
    crate::code_rule_dsl::logic::profile::set_directory(&prof);
    findings(&whole(), &dir);
    let lines = decided(&prof, &["callable candidates"]);
    assert!(lines.iter().any(|l| l.contains("same_globals bbbf by file (")), "{lines:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The asks of a program's exported relations are recorded reader by reader (`RERAIL_ASKS`): `unsafe_pair` reads
/// `same_globals` with its file and both functions bound (`bbbf`), so its keys and rows used are two relations
/// named for that shape, and outputs, with the files answering each key (its document column names them); `runs_in`
/// reads `calls_in` with nothing bound.
#[test]
fn the_asks_of_exported_relations_are_recorded_by_shape() {
    use crate::code_rule_dsl::logic::plan::corpus::preparation::asks;
    // With the run's `doc` (a document's path), which a corpus run's programs read.
    let program = format!("{}.decl doc(d: number, p: symbol, q: symbol)\n.input doc\n", whole());
    let v = crate::code_rule_dsl::logic::rr::parse_version(&program).unwrap();
    let pre = crate::code_rule_dsl::typing::file_local::programs_of(v.clone()).unwrap();
    let got = asks::recorded(&v, &pre.file_local, &pre.locations).unwrap();
    assert!(got.unlocated.is_empty(), "every relation asked by key names its file: {:?}", got.unlocated);
    assert!(got.located.iter().any(|(tag, how)| tag.ends_with("__same_globals__unsafe_pair__bbbf") && how == "document column 0, key 0"), "its key names its file: {:?}", got.located);
    let out = got.version.expect("asks recorded");
    let outputs: Vec<String> = out.outputs().map(|o| out.text(o).to_owned()).collect();
    let named = |prefix: &str, tail: &str| outputs.iter().any(|o| o.starts_with(prefix) && o.ends_with(tail));
    assert!(named(asks::KEY, "__same_globals__unsafe_pair__bbbf"), "{outputs:?}");
    assert!(named(asks::USED, "__same_globals__unsafe_pair__bbbf"), "{outputs:?}");
    assert!(named(asks::USED, "__calls_in__runs_in__fff"), "{outputs:?}");
    assert!(!named(asks::KEY, "__calls_in__runs_in__fff"), "no keys where none is bound: {outputs:?}");
    // `same_globals` names its file by its own document column, no relay: its asked keys' files, and its index.
    assert!(named(asks::FILES, "__same_globals__unsafe_pair__bbbf"), "{outputs:?}");
    assert!(outputs.iter().any(|o| o == &format!("{}same_globals__bbbf", asks::PAIRS)), "{outputs:?}");
    assert!(outputs.iter().any(|o| o == "unsafe_pair"), "the program's own outputs stay");
}
