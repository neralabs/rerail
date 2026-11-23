//! core.rerail's base relations the corpus runner binds beyond T's (`facts::vtab::syntax::Extra`), each against W's
//! notion of it (`CodeRuleDslRuntime`, the matcher, the regex query) on fixture files.

use rerail_blocks::vector::Vector;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::model::{RegexLines, RegexOptions};
// cratify: a re-export of rerail-engine's, which its facades of this module make
use crate::code_rule_dsl::typing::file_local::Binding;
use crate::code_rule_dsl::test_support::{CAPABILITIES, clause_document, compile_document_with, evaluate_file, violation};
use serde_json::{Value, json};
use crate::code_rule_dsl::logic::plan::output::Outputs;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// A row of values, each as the outputs show it (`Display`).
macro_rules! r {
    ($($v:expr),* $(,)?) => { vec![$($v.to_string()),*] };
}

/// `files` (path, source) written under a fresh directory named for `name`.
fn corpus(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension(format!("base-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    for (path, source) in files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, source).unwrap();
    }
    dir
}

/// `program`'s run over the corpus in `dir`: its outputs, read where they are.
fn run_outputs(dir: &std::path::Path, program: &str) -> Outputs {
    let out = dir.join(".out");
    super::run(program, dir, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    Outputs::open(&out).unwrap()
}

/// `outputs`' relations `rels`: each one's rows, each value as shown (`Relation::shown`), none where it is absent.
fn shown_of(outputs: &Outputs, rels: &[&str]) -> Vec<Vec<Vec<String>>> {
    rels.iter().map(|rel| outputs.get(rel).map(|r| r.shown()).unwrap_or_default()).collect()
}

/// `program`'s outputs over the corpus in `dir`, by relation: each `<rel>`'s rows, each value as shown.
fn run(dir: &std::path::Path, program: &str, outputs: &[&str]) -> Vec<Vec<Vec<String>>> {
    shown_of(&run_outputs(dir, program), outputs)
}

/// Rows as the outputs show a relation: sorted, each once.
fn sorted(rows: impl IntoIterator<Item = Vec<String>>) -> Vec<Vec<String>> {
    rows.into_iter().collect::<BTreeSet<_>>().into_iter().collect()
}

/// W's findings of one Python `clause` (its `forbid`, at `violation()`) in `source`.
fn w(forbid: Value, path: &str, source: &str) -> Vec<Value> {
    w_as(forbid, "python", path, source)
}

/// W's findings of one clause in `source`, a file of language `language` (one with no grammar: "text").
fn w_as(forbid: Value, language: &str, path: &str, source: &str) -> Vec<Value> {
    w_reporting(forbid, violation(), language, path, source)
}

/// `w_as` with the clause's `violation` (its anchor: `capture.NAME` reports a regex group's span).
fn w_reporting(forbid: Value, violation: Value, language: &str, path: &str, source: &str) -> Vec<Value> {
    let capabilities: Value = serde_json::from_str(CAPABILITIES).unwrap();
    let compiled = compile_document_with(clause_document(json!({ "id": "c", "forbid": forbid, "violation": violation })), &capabilities)
        .unwrap_or_else(|e| panic!("{}", e.reason));
    evaluate_file(&compiled, language, path, source)["violations"].as_array().unwrap().clone()
}

/// W's spans of a content regex's findings in `source` (`lines`: a `lines` regex's), each as `path`, start line and
/// end line (1-based), start column and, with `text`, the text matched.
fn w_regex(pattern: &str, lines: bool, language: &str, path: &str, source: &str, text: bool) -> Vec<Vec<String>> {
    let mut regex = json!({ "source": "file.content", "pattern": pattern });
    if lines {
        regex["lines"] = json!({});
    }
    w_as(json!({ "regex": regex }), language, path, source)
        .into_iter()
        .map(|v| {
            let at = |k: &str| v["span"][k].as_u64().unwrap() as usize;
            let mut row = r![path, at("startLine") + 1, at("endLine") + 1, at("startColumn")];
            if text {
                // The span's bytes are the file's, in a line too (its line and column are the line's).
                row.push(source[at("startByte")..at("endByte")].to_owned());
            }
            row
        })
        .collect()
}

/// Every terminator `str.splitlines()` knows, a `\r\n`, a blank line, and no final terminator.
const LINES: &str = "a = 1\r\nb = 2\rc = 3\x0bd = 4\x0ce = 5\x1cf\x1dg\x1eh\u{85}i\u{2028}j\u{2029}k = 6\n\n  last = 0";

/// W's lines of a `lines` regex (`query::regex_query_matches`), 1-based, with their text.
fn w_lines(source: &str, lf_only: bool) -> Vec<Vec<String>> {
    let options = RegexOptions {
        lines: Some(RegexLines { include: vec![], exclude: vec![], first: true, lf_only, state: None, indentation: None }),
        ..Default::default()
    };
    let (matches, complete) = crate::code_rule_dsl::query::regex_query_matches(&regex::Regex::new("(?s).*").unwrap(), &crate::code_rule_dsl::model::RegexSource::Content, source, &options, usize::MAX);
    assert!(complete);
    matches.into_iter().map(|m| r![m.line.unwrap() + 1, &source[m.context]]).collect()
}

/// `src_line` is the file's lines as its language ends them (Python: `\r\n`, `\r`, `\n`; the facts' `LineTable`, decision
/// 0612), `line_count` the lines a linter counts; a line span's `text` is the line without its terminator, its `line`
/// and `end_line` its number, its `col` 0. W's lines of a `lines` regex (Python's `str.splitlines()`, which also ends
/// a line at `\v`, `\f`, `\x1c`-`\x1e`, `\x85`, U+2028 and U+2029) are the library's `py_line`, made from them.
#[test]
fn source_lines_are_the_languages_lines() {
    let dir = corpus("lines", &[("a.py", LINES), ("b.py", "x = 1\n")]);
    let program = r#"
#include "core.rerail"
#include "data-layer/pub-lines.rerail"
.decl lines(p: symbol, l: number, s: symbol)
lines(p, l, s) :- doc(d, p, _), src_line(d, l, sp), text(sp, s), line(sp, l), end_line(sp, l), col(sp, 0).
.decl split(l: number, s: symbol)
split(l, s) :- doc(d, "a.py", _), py_line(d, l, s).
.decl count(p: symbol, k: number)
count(p, k) :- doc(d, p, _), line_count(d, k).
.output lines, split, count
"#;
    let got = run(&dir, program, &["lines", "split", "count"]);
    let table = |path: &str, source: &str| {
        let t = LineTable::new(source, crate::code_rule_dsl::lines::terminators(GrammarDialect::Python));
        (0..t.count()).map(|i| r![path, i + 1, &source[t.start(i)..t.end(i, source.as_bytes())]]).collect::<Vec<_>>()
    };
    let want = sorted(table("a.py", LINES).into_iter().chain(table("b.py", "x = 1\n")));
    assert!(want.contains(&r!["a.py", "3", "c = 3\x0bd = 4\x0ce = 5\x1cf\x1dg\x1eh\u{85}i\u{2028}j\u{2029}k = 6"]), "{want:?}");
    assert!(want.contains(&r!["b.py", "2", ""]), "the empty line after a final terminator: {want:?}");
    assert_eq!(got[0], want);
    let python = w_lines(LINES, false);
    assert_eq!(python.len(), 13, "{python:?}");
    assert_eq!(got[1], sorted(python));
    assert_eq!(got[2], [r!["a.py", 5], r!["b.py", 1]]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A `lines` regex clause as `py_line` (the library's `str.splitlines()` lines, over `src_line`) and `match`: W's
/// findings, line for line, a form feed ending a line as `splitlines()` ends it.
#[test]
fn a_lines_regex_is_py_line_and_match() {
    let files = [("a.py", "x = needle\n# needle\n  y = needle; needle\r\nz = 1\x0cneedle = 2\n"), ("b/c.py", "no = 1\n")];
    let dir = corpus("lines-regex", &files);
    let program = r#"
#include "core.rerail"
#include "data-layer/pub-lines.rerail"
.decl hit(p: symbol, l: number)
hit(p, l) :- doc(d, p, _), py_line(d, l, s), match(".*needle.*", s), !match("\\s*#.*", s).
.output hit
"#;
    let got = run(&dir, program, &["hit"]);
    let forbid = json!({ "regex": { "source": "file.content", "pattern": "needle", "lines": { "exclude": ["^\\s*#"] } } });
    let want = files.iter().flat_map(|(path, source)| {
        w(forbid.clone(), path, source).into_iter().map(move |v| r![path, v["span"]["startLine"].as_u64().unwrap() + 1])
    });
    let want = sorted(want);
    assert_eq!(want, [["a.py", "1"], ["a.py", "3"], ["a.py", "5"]]);
    assert_eq!(got[0], want);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `source(d, sp)`: the whole file, its text exactly as W's content regex reads it; its lines 1 to the line holding
/// its end.
#[test]
fn source_is_the_files_text() {
    let files = [("a.py", "import os\n\n\tx = 1\r\n"), ("b.py", "import os\nx = 1\n"), ("c.py", "")];
    let dir = corpus("source", &files);
    let program = r#"
.decl doc(d: number, path: symbol, module: symbol)
.decl source(d: number, sp: span)
.decl text(n: loc, s: symbol)
.decl line(n: loc, l: number)
.decl end_line(n: loc, l: number)
.input doc, source, text, line, end_line
.decl hit(p: symbol)
hit(p) :- doc(d, p, _), source(d, sp), text(sp, s), match("(?s).*import os\n\\s+x = 1\r\n", s).
.decl lines(p: symbol, l: number, e: number)
lines(p, l, e) :- doc(d, p, _), source(d, sp), line(sp, l), end_line(sp, e).
.output hit, lines
"#;
    let got = run(&dir, program, &["hit", "lines"]);
    let forbid = json!({ "regex": { "source": "file.content", "pattern": "import os\\n\\s+x = 1\\r\\n\\z" } });
    let want = sorted(files.iter().filter(|(path, source)| !w(forbid.clone(), path, source).is_empty()).map(|(path, _)| r![path]));
    assert_eq!(want, [["a.py"]]);
    assert_eq!(got[0], want);
    // W's span of the whole file (`\A(?s:.*)\z`): its end line is the line holding the end.
    let whole = json!({ "regex": { "source": "file.content", "pattern": "\\A(?s:.*)\\z" } });
    let want = sorted(files.iter().map(|(path, source)| {
        let [v] = w(whole.clone(), path, source).try_into().unwrap();
        r![path, v["span"]["startLine"].as_u64().unwrap() + 1, v["span"]["endLine"].as_u64().unwrap() + 1]
    }));
    assert_eq!(got[1], want);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `src_line(d, l, sp)`: the lines `line(n, l)` numbers (the facts' line table, W's `layout` lines), the empty line
/// after a final terminator included.
#[test]
fn src_lines_are_the_line_table() {
    let source = "a = 1\r\nb = 2\rc = 3\x0cd = 4\n";
    let dir = corpus("tape-lines", &[("a.py", source)]);
    let program = r#"
.decl src_line(d: number, l: number, sp: span)
.decl text(n: loc, s: symbol)
.input src_line, text
.decl lines(l: number, s: symbol)
lines(l, s) :- src_line(_, l, sp), text(sp, s).
.output lines
"#;
    let got = run(&dir, program, &["lines"]);
    let table = LineTable::new(source, crate::code_rule_dsl::lines::terminators(GrammarDialect::Python));
    let want = sorted((0..table.count()).map(|i| r![i + 1, &source[table.start(i)..table.end(i, source.as_bytes())]]));
    assert_eq!(got[0], want);
    assert!(want[0] == ["1", "a = 1"] && want.contains(&r!["4", ""]), "{want:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `node_text`, `line`, `end_line` and `col` of a node: W's span of it (its text, start line, end line and start
/// column), for every node of a kind, the module's included (its text is its own, not the file's).
#[test]
fn node_text_and_positions_are_ws_spans() {
    let files = [
        ("a.py", "\n\nimport os  # c\nif a < b <= c:\n    f(x,\n      y)  # é\n\n"),
        ("b.py", "def g(p: Callable[[int], str]) -> None:\r\n    return h(p)\r\n"),
    ];
    let dir = corpus("node-text", &files);
    let kinds = ["module", "comparison_operator", "call", "type", "argument_list"];
    let program = format!(
        r#"
.decl doc(d: number, path: symbol, module: symbol)
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl node_text(n: node, s: symbol)
.decl line(n: loc, l: number)
.decl end_line(n: loc, l: number)
.decl col(n: loc, c: number)
.input doc, node, node_text, line, end_line, col
.decl kinds(k: symbol)
{}
.decl spans(p: symbol, k: symbol, l: number, e: number, c: number, s: symbol)
spans(p, k, l, e, c, s) :- doc(d, p, _), node(d, n, k, _, _), kinds(k), line(n, l), end_line(n, e), col(n, c), node_text(n, s).
.output spans
"#,
        kinds.iter().map(|k| format!("kinds({k:?}).")).collect::<Vec<_>>().join("\n")
    );
    let got = run(&dir, &program, &["spans"]);
    let mut want = Vec::new();
    for (path, source) in files {
        for kind in kinds {
            for v in w(json!({ "ast": { "language": "python", "rule": { "kind": kind } } }), path, source) {
                let span = &v["span"];
                let at = |k: &str| span[k].as_u64().unwrap();
                want.push(r![path, kind, at("startLine") + 1, at("endLine") + 1, at("startColumn"), &source[at("startByte") as usize..at("endByte") as usize]]);
            }
        }
    }
    assert!(want.iter().any(|r| r[..3] == ["a.py", "module", "3"]), "the module starts at its first token: {want:?}");
    assert_eq!(got[0], sorted(want));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A program that reads none of them binds no span: `line` stays the node view, `text` T's.
#[test]
fn without_spans_text_and_line_are_ts() {
    let dir = corpus("no-spans", &[("a.py", "x = 1\ny = 2\n")]);
    let program = r#"
.decl text(n: node, s: symbol)
.decl line(n: node, l: number)
.input text, line
.decl t(s: symbol, l: number)
t(s, l) :- text(n, s), line(n, l).
.output t
"#;
    assert_eq!(run(&dir, program, &["t"])[0], [["x", "1"], ["y", "2"]]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file whose tree starts with an ERROR node (its kind atom past the dialect's stride) is read in its own dialect:
/// `node` has its ERROR nodes, by kind as W's `kind: "ERROR"` finds them, and `syntax_error` has the file.
#[test]
fn a_tree_rooted_at_an_error_node_is_read() {
    let (path, source) = ("graphql/d.py", "def f(scope: PlanScope):\n    return (\n");
    let dir = corpus("error-root", &[(path, source)]);
    let program = r#"
.decl doc(d: number, path: symbol, module: symbol)
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl line(n: node, l: number)
.decl syntax_error(d: number)
.input doc, node, line, syntax_error
.decl errors(p: symbol, l: number, s: number, e: number)
errors(p, l, s, e) :- doc(d, p, _), node(d, n, "ERROR", s, e), line(n, l).
.decl invalid(p: symbol)
invalid(p) :- doc(d, p, _), syntax_error(d).
.output errors, invalid
"#;
    let got = run(&dir, program, &["errors", "invalid"]);
    let want = sorted(w(json!({ "ast": { "language": "python", "syntax": "invalid", "rule": { "kind": "ERROR" } } }), path, source).into_iter().map(|v| {
        let span = &v["span"];
        r![path, span["startLine"].as_u64().unwrap() + 1, span["startByte"], span["endByte"]]
    }));
    assert!(want[0][..3] == [path, "1", "0"], "W finds the ERROR root: {want:?}");
    assert_eq!(got[0], want);
    assert_eq!(got[1], [[path]]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A base relation a program reads is an input though its `.input` leaves it out, as in an authored file: `text`
/// over spans only (the program port-21 reported failing with "no such column: n in SELECT n, s FROM text"), in
/// rules run per chunk and in rules run once over every file.
#[test]
fn base_relations_read_are_inputs() {
    let files = [("graphql/bad.py", "def f(:\n    ServerPlan\n"), ("graphql/ok.py", "x = 1\n")];
    let dir = corpus("span-text", &files);
    let program = r#"
.decl doc(d: number, path: symbol, module: symbol)
.decl source(d: number, sp: span)
.decl text(n: loc, s: symbol)
.decl syntax_error(d: number)
.input doc, syntax_error, source
.decl hit(p: symbol)
hit(p) :- doc(d, p, _), syntax_error(d), source(d, sp), text(sp, s), contains("ServerPlan", s).
.decl plain(p: symbol)
plain(p) :- doc(d, p, _), !syntax_error(d), source(d, sp), text(sp, s), contains("x = 1", s).
.decl same(p: symbol, q: symbol)
same(p, q) :- doc(d, p, _), source(d, sp), text(sp, s), doc(e, q, _), source(e, sq), text(sq, s).
.output hit, plain, same
"#;
    let got = run(&dir, program, &["hit", "plain", "same"]);
    assert_eq!(got[0], [["graphql/bad.py"]]);
    assert_eq!(got[1], [["graphql/ok.py"]]);
    assert_eq!(got[2], sorted(files.iter().map(|(p, _)| r![p, p])));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A corpus with no documents (no files at all, or only files the runner skips) gives no rows, not an error; so
/// does a program declaring an input no rule reads.
#[test]
fn no_documents_give_no_rows() {
    let program = r#"
.decl doc(d: number, path: symbol, module: symbol)
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl source(d: number, sp: span)
.decl src_line(d: number, l: number, sp: span)
.decl text(n: loc, s: symbol)
.input doc, node, source, src_line, text
.decl calls(p: symbol)
calls(p) :- doc(d, p, _), node(d, _, "call", _, _).
.decl lines(p: symbol, s: symbol)
lines(p, s) :- doc(d, p, _), src_line(d, _, sp), text(sp, s).
.output calls, lines
"#;
    let empty = corpus("no-docs-empty", &[]);
    std::fs::create_dir_all(&empty).unwrap();
    let skipped = corpus("no-docs-skipped", &[(".venv/lib/a.py", "f()\n"), (".git/hooks/b.py", "g()\n")]);
    for dir in [&empty, &skipped] {
        assert_eq!(run(dir, program, &["calls", "lines"]), [Vec::<Vec<String>>::new(), Vec::new()], "{}", dir.display());
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// `match_all(re, sp, m)`: each match of `re` in span `sp`'s text, leftmost-first and non-overlapping, a span whose
/// `line`, `end_line`, `col` and `text` are W's finding of a content regex (the tape's lines) over `source`, and of a
/// `lines` regex over `src_line`; matches across lines, CRLF and non-ASCII text included.
#[test]
fn match_all_is_ws_regex() {
    let files = [
        ("a.py", "x = 'needle needles'\r\ny = 1  # néedle needle_2\n\tneedleneedle = 2\nz = \"\"\"needle\nend\"\"\"\n"),
        ("b.py", "aaaa = 'aaaaa'\n"),
    ];
    let dir = corpus("match-all", &files);
    let program = r#"
.decl doc(d: number, path: symbol, module: symbol)
.decl source(d: number, sp: span)
.decl src_line(d: number, l: number, sp: span)
.decl text(n: loc, s: symbol)
.decl line(n: loc, l: number)
.decl end_line(n: loc, l: number)
.decl col(n: loc, c: number)
.input doc, source, src_line, text, line, end_line, col
.decl whole(p: symbol, l: number, e: number, c: number, s: symbol)
whole(p, l, e, c, s) :- doc(d, p, _), source(d, sp), match_all("needle\\w*", sp, m), line(m, l), end_line(m, e), col(m, c), text(m, s).
.decl across(p: symbol, l: number, e: number, c: number)
across(p, l, e, c) :- doc(d, p, _), source(d, sp), match_all("(?s)needle.end", sp, m), line(m, l), end_line(m, e), col(m, c).
.decl lines(p: symbol, l: number, e: number, c: number, s: symbol)
lines(p, l, e, c, s) :- doc(d, p, _), src_line(d, k, sp), match_all("needle\\w*", sp, m), line(m, l), end_line(m, e), col(m, c), text(m, s), l = k.
.decl pairs(p: symbol, l: number, e: number, c: number, s: symbol)
pairs(p, l, e, c, s) :- doc(d, p, _), source(d, sp), match_all("aa", sp, m), line(m, l), end_line(m, e), col(m, c), text(m, s).
.decl none(p: symbol)
none(p) :- doc(d, p, _), source(d, sp), !match_all("needle", sp, _).
.output whole, across, lines, pairs, none
"#;
    let got = run(&dir, program, &["whole", "across", "lines", "pairs", "none"]);
    let all = |pattern: &str, lines: bool, text: bool| sorted(files.iter().flat_map(|(path, source)| w_regex(pattern, lines, "python", path, source, text)));
    let whole = all("needle\\w*", false, true);
    assert_eq!(whole.len(), 5, "{whole:?}");
    assert_eq!(got[0], whole);
    let across = all("(?s)needle.end", false, false);
    assert_eq!(across, [["a.py", "4", "5", "7"]]);
    assert_eq!(got[1], across);
    assert_eq!(got[2], all("needle\\w*", true, true));
    let pairs = all("aa", false, true);
    assert_eq!(pairs, [["b.py", "1", "1", "0", "aa"], ["b.py", "1", "1", "10", "aa"], ["b.py", "1", "1", "2", "aa"], ["b.py", "1", "1", "8", "aa"]]);
    assert_eq!(got[3], pairs);
    assert_eq!(got[4], [["b.py"]]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file with no grammar that `.scope text` reads (`.md`, `.rst`, `.txt`, …) is a text-only document: `doc`, `source`,
/// `src_line`, `line_count` and `match_all`, each as W reads a file of no language (its lines `\r\n`, `\r`, `\n`,
/// U+2028 and U+2029), and no node. A file that is not UTF-8 is no document; a hidden one neither.
#[test]
fn text_files_are_text_only_documents() {
    let rst = "Graph query\r\n===========\n\nUse graph_query_builder( no more.\u{2028}needle here\nlast needle";
    let md = "# Title\n\nneedle\n";
    let files = [("docs/graph_query.rst", rst), ("docs/design/a.md", md), ("notes.txt", ""), ("pkg/m.py", "f(needle)\n"), (".hidden.md", "needle\n")];
    let dir = corpus("text-docs", &files);
    std::fs::write(dir.join("docs/logo.png"), [0x89, b'P', b'N', b'G', 0xff, 0xfe, 0x00]).unwrap();
    let program = r#"
.scope text "**/*.md", "**/*.rst", "**/*.txt"
.decl doc(d: number, path: symbol, module: symbol)
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl source(d: number, sp: span)
.decl src_line(d: number, l: number, sp: span)
.decl line_count(d: number, k: number)
.decl text(n: loc, s: symbol)
.decl line(n: loc, l: number)
.decl end_line(n: loc, l: number)
.decl col(n: loc, c: number)
.decl docs(p: symbol)
docs(p) :- doc(_, p, _).
.decl treed(p: symbol)
treed(p) :- doc(d, p, _), node(d, _, _, _, _).
.decl advertised(p: symbol)
advertised(p) :- doc(d, p, _), source(d, sp), text(sp, s), contains("graph_query_builder(", s).
.decl lines(p: symbol, l: number, s: symbol)
lines(p, l, s) :- doc(d, p, _), src_line(d, l, sp), text(sp, s).
.decl counts(p: symbol, k: number)
counts(p, k) :- doc(d, p, _), line_count(d, k).
.decl hits(p: symbol, l: number, e: number, c: number, s: symbol)
hits(p, l, e, c, s) :- doc(d, p, _), source(d, sp), match_all("needle", sp, m), line(m, l), end_line(m, e), col(m, c), text(m, s).
.output docs, treed, advertised, lines, counts, hits
"#;
    let got = run(&dir, program, &["docs", "treed", "advertised", "lines", "counts", "hits"]);
    let texts = [("docs/design/a.md", md), ("docs/graph_query.rst", rst), ("notes.txt", "")];
    assert_eq!(got[0], sorted(["docs/design/a.md", "docs/graph_query.rst", "notes.txt", "pkg/m.py"].map(|p| r![p])));
    assert_eq!(got[1], [["pkg/m.py"]]);
    assert_eq!(got[2], [["docs/graph_query.rst"]]);
    let table = |path: &str, source: &str| {
        let t = LineTable::new(source, Terminators::Eslint);
        (0..t.count()).map(|i| r![path, i + 1, &source[t.start(i)..t.end(i, source.as_bytes())]]).collect::<Vec<_>>()
    };
    let python = LineTable::new("f(needle)\n", crate::code_rule_dsl::lines::terminators(GrammarDialect::Python));
    let python = (0..python.count()).map(|i| r!["pkg/m.py", i + 1, &"f(needle)\n"[python.start(i)..python.end(i, b"f(needle)\n")]]);
    assert_eq!(got[3], sorted(texts.iter().flat_map(|(p, s)| table(p, s)).chain(python)));
    assert!(got[3].contains(&r!["docs/graph_query.rst", "5", "needle here"]), "U+2028 ends a line: {:?}", got[3]);
    // The lines a linter counts: the empty line after a final terminator left out, as W's.
    assert_eq!(got[4], sorted(texts.iter().map(|(p, s)| r![p, w_lines(s, false).len()]).chain([r!["pkg/m.py", 1]])));
    let hits = texts.iter().flat_map(|(p, s)| w_regex("needle", false, "text", p, s, true)).chain(w_regex("needle", false, "python", "pkg/m.py", "f(needle)\n", true));
    let hits = sorted(hits);
    assert_eq!(hits.len(), 4, "{hits:?}");
    assert_eq!(got[5], hits);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `match(p, s)` with `p` computed in the rule is the same whole-string test as with a constant pattern, each
/// distinct (pattern, string) tested once; and the program printed (`rr::print`, the overlay's merge) runs alike.
#[test]
fn a_computed_pattern_is_matched_per_row() {
    let source = "def load(): pass\ndef load_async(): pass\ndef loader(): pass\nx = load_lazy\ny = re_load_x\n";
    let dir = corpus("computed-pattern", &[("a.py", source)]);
    let program = r#"
.decl name(n: node, s: symbol)
.decl twin(s: symbol, t: symbol)
twin(s, t) :- name(_, s), name(_, t), x = cat(s, "_[a-z]+"), match(x, t).
.decl lone(s: symbol)
lone(s) :- name(_, s), x = cat(s, "_[a-z]+"), !match(x, "load_async").
.output twin, lone
"#;
    let names = ["load", "load_async", "loader", "load_lazy", "x", "y", "re_load_x"];
    let whole = |p: &str, s: &str| regex::Regex::new(&format!("^(?:{p})$")).unwrap().is_match(s);
    let twin = sorted(names.iter().flat_map(|s| names.iter().filter(move |t| whole(&format!("{s}_[a-z]+"), t)).map(move |t| r![s, t])));
    assert_eq!(twin, [["load", "load_async"], ["load", "load_lazy"]]);
    let lone = sorted(names.iter().filter(|s| !whole(&format!("{s}_[a-z]+"), "load_async")).map(|s| r![s]));
    let printed = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_version(program).unwrap()).unwrap();
    for program in [program, printed.as_str()] {
        assert_eq!(run(&dir, program, &["twin", "lone"]), [twin.clone(), lone.clone()], "{program}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// `match_group(re, sp, m, g, gsp)`: per match `m` of `re` in `sp`, the span `gsp` of each named group `g` that takes
/// part in it, placed as W places a finding anchored at that capture (`capture.NAME`), in `source` and in `src_line`;
/// `m` is `match_all`'s. The regex is internal-dead-code's dynamic-import shape.
#[test]
fn match_group_is_ws_capture() {
    let re = r#"(?:\bimport_module|__import__)\(\s*['"](?P<module>[^'"]+)['"]\s*\)(?:\.(?P<attr>[A-Za-z_]\w*))?"#;
    let files = [
        ("a.py", "import importlib\nm = importlib.import_module('pkg.mod').run\r\nn = __import__(\"other\")  # é\nx = import_module( 'a.b' ).c; y = __import__('z').w\n"),
        ("b.py", "none = 1\n"),
    ];
    let dir = corpus("match-group", &files);
    let quoted = re.replace('\\', "\\\\").replace('"', "\\\"");
    let program = r#"
.decl doc(d: number, path: symbol, module: symbol)
.decl source(d: number, sp: span)
.decl src_line(d: number, l: number, sp: span)
.decl text(n: loc, s: symbol)
.decl line(n: loc, l: number)
.decl end_line(n: loc, l: number)
.decl col(n: loc, c: number)
.decl groups(p: symbol, g: symbol, l: number, e: number, c: number, s: symbol)
groups(p, g, l, e, c, s) :- doc(d, p, _), source(d, sp), match_group("RE", sp, _, g, gsp), line(gsp, l), end_line(gsp, e), col(gsp, c), text(gsp, s).
.decl lines(p: symbol, g: symbol, l: number, e: number, c: number, s: symbol)
lines(p, g, l, e, c, s) :- doc(d, p, _), src_line(d, _, sp), match_group("RE", sp, _, g, gsp), line(gsp, l), end_line(gsp, e), col(gsp, c), text(gsp, s).
.decl imports(p: symbol, l: number, s: symbol, a: symbol)
imports(p, l, s, a) :- doc(d, p, _), source(d, sp), match_all("RE", sp, m), line(m, l), match_group("RE", sp, m, "module", ms), text(ms, s), match_group("RE", sp, m, "attr", asp), text(asp, a).
.decl bare(p: symbol, s: symbol)
bare(p, s) :- doc(d, p, _), source(d, sp), match_group("RE", sp, m, "module", ms), text(ms, s), !match_group("RE", sp, m, "attr", _).
.output groups, lines, imports, bare
"#
    .replace("RE", &quoted);
    let got = run(&dir, &program, &["groups", "lines", "imports", "bare"]);
    // W's finding at a capture, per group; where the group took no part W falls back to the whole match, so those are
    // left out (the whole match holds the call's parenthesis).
    let w_groups = |lines: bool| {
        let mut regex = json!({ "source": "file.content", "pattern": re });
        if lines {
            regex["lines"] = json!({});
        }
        sorted(files.iter().flat_map(|(path, source)| {
            let regex = regex.clone();
            ["module", "attr"].into_iter().flat_map(move |g| {
                let v = json!({ "anchor": format!("capture.{g}"), "code": "X", "message": "x" });
                w_reporting(json!({ "regex": regex.clone() }), v, "python", path, source).into_iter().filter_map(move |v| {
                    let at = |k: &str| v["span"][k].as_u64().unwrap() as usize;
                    let s = &source[at("startByte")..at("endByte")];
                    (!s.contains('(')).then(|| r![path, g, at("startLine") + 1, at("endLine") + 1, at("startColumn"), s])
                })
            })
        }))
    };
    let want = w_groups(false);
    assert_eq!(want.len(), 7, "{want:?}");
    assert_eq!(got[0], want);
    assert_eq!(got[1], w_groups(true));
    assert_eq!(got[2], [["a.py", "2", "pkg.mod", "run"], ["a.py", "4", "a.b", "c"], ["a.py", "4", "z", "w"]]);
    assert_eq!(got[3], [["a.py", "other"]]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Python literals and folds W's `value` reads: escapes, adjacent literals, numbers in every base and form,
/// operators, a bool, None, Ellipsis, containers, bytes, f-strings with and without an interpolation, and names
/// (which only a following reading knows).
const VALUES: &str = r#"import os
A = "k"
B = A
x = "a\x41\u00e9" 'b'
y = r"\d+" "z"
n = 0xff + 1_000
f1 = 1.5
f2 = 1e20
f3 = 10 / 4
t = True
nn = None
e = ...
neg = -3
p = 2 ** 10
s3 = "ab" * 2
tup = (1, "a")
lst = [1]
bb = b"x"
fs = f"x{os.sep}"
fz = f"plain"
call(A, B, x)
def g(A=1):
    return A
twelve = "12"
n12 = 12
d = {"a": 1}
st = {1}
c = 1j
"#;

/// Python's type names of the scalars `value` gives (`value::python::to_str`), as W's `value.type` names them.
const SCALARS: [&str; 6] = ["str", "int", "float", "bool", "NoneType", "ellipsis"];

/// W's value of the node spanning `[start, end)` as `v` (a scalar of type `ty`, its `str`), by W's `value.eq`;
/// `follow`: names followed. An Ellipsis has no JSON literal: its type alone says it.
fn w_value_is(path: &str, source: &str, start: u64, end: u64, ty: &str, v: &str, follow: bool) -> bool {
    let eq = match ty {
        "str" => json!(v),
        "int" => json!(v.parse::<i64>().unwrap()),
        "float" => json!(v.parse::<f64>().unwrap()),
        "bool" => json!(v == "True"),
        "NoneType" => Value::Null,
        _ => return v == "Ellipsis",
    };
    let rule = json!({ "value": { "type": ty, "eq": eq, "follow": follow } });
    w(json!({ "ast": { "language": "python", "rule": rule } }), path, source)
        .iter()
        .any(|f| f["span"]["startByte"].as_u64() == Some(start) && f["span"]["endByte"].as_u64() == Some(end))
}

/// W's nodes whose value is a scalar of each type (`value.type`, `follow` or not), as `(start, end, type)`.
fn w_scalars(path: &str, source: &str, follow: bool) -> BTreeSet<(u64, u64, String)> {
    let mut out = BTreeSet::new();
    for ty in SCALARS {
        let rule = json!({ "value": { "type": ty, "follow": follow } });
        for f in w(json!({ "ast": { "language": "python", "rule": rule } }), path, source) {
            out.insert((f["span"]["startByte"].as_u64().unwrap(), f["span"]["endByte"].as_u64().unwrap(), ty.to_owned()));
        }
    }
    out
}

/// `outputs`' rows of `rel(s, e, v)` (two numbers and a symbol) as `(start, end, v)`.
fn spans_and_values(outputs: &Outputs, rel: &str) -> Vec<(u64, u64, String)> {
    let number = |v: crate::code_rule_dsl::logic::plan::output::Value<'_>| u64::try_from(v.int().expect("a number")).unwrap();
    outputs.rows(rel).iter().map(|r| (number(r.get(0)), number(r.get(1)), r.get(2).text().expect("a symbol").to_owned())).collect()
}

/// `value(n, v)` is W's value of each expression node, as Python's `str(v)`: a row exactly where W's value is a known
/// scalar (a str, int, float, bool, None or Ellipsis: `value.type`), and that row's `v` is W's value (`value.eq`, the
/// type fixed). A name, an f-string with an interpolation, a container and bytes have none; escapes are decoded and
/// adjacent literals joined.
#[test]
fn value_is_ws_value() {
    let dir = corpus("value", &[("a.py", VALUES)]);
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl value(n: node, v: symbol)
.input node, value
.decl vals(s: number, e: number, v: symbol)
vals(s, e, v) :- value(n, v), node(_, n, _, s, e).
.output vals
"#;
    let got = spans_and_values(&run_outputs(&dir, program), "vals");
    let text = |s: u64, e: u64| &VALUES[s as usize..e as usize];
    let scalars = w_scalars("a.py", VALUES, false);
    assert_eq!(got.iter().map(|(s, e, _)| (*s, *e)).collect::<BTreeSet<_>>(), scalars.iter().map(|(s, e, _)| (*s, *e)).collect(), "{got:?}");
    for (start, end, ty) in &scalars {
        let (_, _, v) = got.iter().find(|(s, e, _)| (s, e) == (start, end)).unwrap();
        assert!(w_value_is("a.py", VALUES, *start, *end, ty, v, false), "{:?} is not W's {ty} {v:?}", text(*start, *end));
    }
    let value_of = |source: &str| got.iter().filter(|(s, e, _)| text(*s, *e) == source).map(|(_, _, v)| v.as_str()).collect::<Vec<_>>();
    assert_eq!(value_of(r#""a\x41\u00e9" 'b'"#), ["aAéb"]);
    assert_eq!(value_of(r#"r"\d+" "z""#), [r"\d+z"]);
    assert_eq!(value_of("0xff + 1_000"), ["1255"]);
    assert_eq!(value_of("1e20"), ["1e+20"]);
    assert_eq!(value_of("10 / 4"), ["2.5"]);
    assert_eq!(value_of("True"), ["True"]);
    assert_eq!(value_of("None"), ["None"]);
    assert_eq!(value_of("..."), ["Ellipsis"]);
    assert_eq!(value_of("-3"), ["-3"]);
    assert_eq!(value_of("2 ** 10"), ["1024"]);
    assert_eq!(value_of(r#""ab" * 2"#), ["abab"]);
    assert_eq!(value_of(r#"f"plain""#), ["plain"]);
    for none in ["A", r#"f"x{os.sep}""#, r#"(1, "a")"#, "[1]", r#"b"x""#] {
        assert_eq!(value_of(none), Vec::<&str>::new(), "{none}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// `static_value(n, v)` is W's value with names followed (`value.follow`): a row exactly where that value is a known
/// scalar, its `v` W's there; a name bound once in the file to a literal (or to such a name) has the literal's value,
/// a parameter none, and a literal the value `value` gives it.
#[test]
fn static_value_is_ws_followed_value() {
    let dir = corpus("static-value", &[("a.py", VALUES)]);
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl value(n: node, v: symbol)
.decl static_value(n: node, v: symbol)
.input node, value, static_value
.decl vals(s: number, e: number, v: symbol)
vals(s, e, v) :- static_value(n, v), node(_, n, _, s, e).
.decl plain(s: number, e: number, v: symbol)
plain(s, e, v) :- value(n, v), node(_, n, _, s, e).
.output vals, plain
"#;
    let out = run_outputs(&dir, program);
    let (got, plain) = (spans_and_values(&out, "vals"), spans_and_values(&out, "plain"));
    let text = |s: u64, e: u64| &VALUES[s as usize..e as usize];
    let scalars = w_scalars("a.py", VALUES, true);
    assert_eq!(got.iter().map(|(s, e, _)| (*s, *e)).collect::<BTreeSet<_>>(), scalars.iter().map(|(s, e, _)| (*s, *e)).collect(), "{got:?}");
    for (start, end, ty) in &scalars {
        let (_, _, v) = got.iter().find(|(s, e, _)| (s, e) == (start, end)).unwrap();
        assert!(w_value_is("a.py", VALUES, *start, *end, ty, v, true), "{:?} is not W's followed {ty} {v:?}", text(*start, *end));
    }
    // Every literal's value is its static value.
    assert!(plain.iter().all(|row| got.contains(row)), "{plain:?}");
    let at = |needle: &str, offset: usize| {
        let start = (VALUES.find(needle).unwrap() + offset) as u64;
        got.iter().filter(|(s, _, _)| *s == start).map(|(_, _, v)| v.as_str()).collect::<Vec<_>>()
    };
    assert_eq!(at("call(A, B, x)", 5), ["k"]);
    assert_eq!(at("call(A, B, x)", 8), ["k"]);
    assert_eq!(at("call(A, B, x)", 11), ["aAéb"]);
    assert_eq!(at("B = A", 4), ["k"]);
    assert_eq!(at("return A", 7), Vec::<&str>::new());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The type names W's `value.type` gives Python values, containers, bytes and complex numbers included.
const TYPES: [&str; 12] = ["str", "int", "float", "bool", "NoneType", "ellipsis", "bytes", "complex", "tuple", "list", "dict", "set"];

/// `value_type_of(n, t)` is W's `value.type` of each node, containers and bytes included; `value_type(v, t)` pairs
/// `value`'s strings with them (the str "12" and the int 12 both `12`); and W's `value.regex`, which reads strings only,
/// is `value`, `value_type_of(n, "str")` and `match`.
#[test]
fn value_types_are_ws_value_types() {
    let dir = corpus("value-type", &[("a.py", VALUES)]);
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl value(n: node, v: symbol)
.decl value_type_of(n: node, t: symbol)
.decl value_type(v: symbol, t: symbol)
.input node, value, value_type_of, value_type
.decl types(s: number, e: number, t: symbol)
types(s, e, t) :- value_type_of(n, t), node(_, n, _, s, e).
.decl pairs(v: symbol, t: symbol)
pairs(v, t) :- value_type(v, t).
.decl hits(s: number, e: number)
hits(s, e) :- value(n, v), value_type_of(n, "str"), match("(?s:.*)(?:b)(?s:.*)", v), node(_, n, _, s, e).
.decl vals(s: number, e: number, v: symbol)
vals(s, e, v) :- value(n, v), node(_, n, _, s, e).
.output types, pairs, hits, vals
"#;
    let outputs = run_outputs(&dir, program);
    let out = shown_of(&outputs, &["types", "pairs", "hits"]);
    let spans = |f: &Value| (f["span"]["startByte"].as_u64().unwrap(), f["span"]["endByte"].as_u64().unwrap());
    let mut want = Vec::new();
    for ty in TYPES {
        for f in w(json!({ "ast": { "language": "python", "rule": { "value": { "type": ty } } } }), "a.py", VALUES) {
            let (s, e) = spans(&f);
            want.push((s, e, ty));
        }
    }
    assert!(want.iter().any(|r| r.2 == "list") && want.iter().any(|r| r.2 == "bytes"), "{want:?}");
    assert_eq!(out[0], sorted(want.iter().map(|(s, e, ty)| r![s, e, ty])));
    // value_type: each typed node's value text with its type.
    let values = spans_and_values(&outputs, "vals");
    let pairs = want.iter().flat_map(|&(s, e, ty)| values.iter().filter(move |(vs, ve, _)| (*vs, *ve) == (s, e)).map(move |(_, _, v)| r![v, ty]));
    let pairs = sorted(pairs);
    assert!(pairs.contains(&r!["12", "str"]) && pairs.contains(&r!["12", "int"]), "{pairs:?}");
    assert_eq!(out[1], pairs);
    let regex = w(json!({ "ast": { "language": "python", "rule": { "value": { "regex": "b" } } } }), "a.py", VALUES);
    let regex = sorted(regex.iter().map(|f| {
        let (s, e) = spans(f);
        r![s, e]
    }));
    assert!(!regex.is_empty());
    assert_eq!(out[2], regex);
    let _ = std::fs::remove_dir_all(&dir);
}

/// W's findings of one `clause` (its `forbid`) in a `language` file.
fn w_in(language: &str, forbid: Value, path: &str, source: &str) -> Vec<Value> {
    let capabilities: Value = serde_json::from_str(CAPABILITIES).unwrap();
    let compiled = compile_document_with(clause_document(json!({ "id": "c", "forbid": forbid, "violation": violation() })), &capabilities)
        .unwrap_or_else(|e| panic!("{}", e.reason));
    evaluate_file(&compiled, language, path, source)["violations"].as_array().unwrap().clone()
}

/// W's comment nodes of `kinds` in a `language` file, as `comments` rows (start line, end line, start column, text
/// and the text without its marker), and the texts without their markers.
fn w_comments(language: &str, kinds: &[&str], path: &str, source: &str) -> (Vec<Vec<String>>, BTreeSet<String>) {
    let (mut rows, mut bodies) = (Vec::new(), BTreeSet::new());
    for kind in kinds {
        for v in w_in(language, json!({ "ast": { "language": language, "rule": { "kind": kind } } }), path, source) {
            let span = &v["span"];
            let at = |k: &str| span[k].as_u64().unwrap();
            let text = &source[at("startByte") as usize..at("endByte") as usize];
            let body = crate::code_rule_dsl::facts::vtab::syntax::comment_body(text);
            rows.push(r![at("startLine") + 1, at("endLine") + 1, at("startColumn"), text, body]);
            bodies.insert(body.to_owned());
        }
    }
    (sorted(rows), bodies)
}

/// `comment(c, s)`: each of W's comment nodes once, `s` its text without the marker; a Python comment, which T's
/// nodes leave out, is a span whose `line`, `end_line`, `col`, `node_text` and `text` are W's span of it, and a Rust
/// comment, which T keeps, is its node.
#[test]
fn comments_are_ws_comment_nodes() {
    let python = "#!/usr/bin/env python\r\n# noqa: E501\nimport os  # type: ignore\ndef f():\n    #\n    return 1  #   trailing   \nx = \"# not a comment\"  # é\n";
    let dir = corpus("comments", &[("a.py", python)]);
    let program = r#"
.decl comment(n: node, s: symbol)
.decl line(n: loc, l: number)
.decl end_line(n: loc, l: number)
.decl col(n: loc, c: number)
.decl node_text(n: node, s: symbol)
.decl text(n: loc, s: symbol)
.input comment, line, end_line, col, node_text, text
.decl comments(l: number, e: number, c: number, t: symbol, s: symbol)
comments(l, e, c, t, s) :- comment(n, s), line(n, l), end_line(n, e), col(n, c), node_text(n, t), text(n, t).
.output comments
"#;
    let got = run(&dir, program, &["comments"]);
    let (want, bodies) = w_comments("python", &["comment"], "a.py", python);
    assert_eq!(bodies, BTreeSet::from(["!/usr/bin/env python", "noqa: E501", "type: ignore", "", "trailing", "é"].map(str::to_owned)), "{want:?}");
    assert_eq!(got[0], want);
    let _ = std::fs::remove_dir_all(&dir);
    let rust = "//! crate doc\n/// item doc\nfn f() { /* block */ let x = 1; // tail\n}\n/** outer block doc */\nfn g() {}\n";
    let dir = corpus("rust-comments", &[("src/lib.rs", rust)]);
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl comment(n: node, s: symbol)
.decl line(n: loc, l: number)
.decl end_line(n: loc, l: number)
.decl col(n: loc, c: number)
.decl node_text(n: node, s: symbol)
.input node, comment, line, end_line, col, node_text
.decl comments(l: number, e: number, c: number, t: symbol, s: symbol)
comments(l, e, c, t, s) :- comment(n, s), node(_, n, _, _, _), line(n, l), end_line(n, e), col(n, c), node_text(n, t).
.output comments
"#;
    let got = run(&dir, program, &["comments"]);
    let (want, bodies) = w_comments("rust", &["line_comment", "block_comment"], "src/lib.rs", rust);
    assert_eq!(bodies, BTreeSet::from(["crate doc", "item doc", "block", "tail", "outer block doc"].map(str::to_owned)), "{want:?}");
    assert_eq!(got[0], want);
    let _ = std::fs::remove_dir_all(&dir);
}

/// W's findings of a Python `ast` rule over `files`, each `path\tline\tcol` (1-based line, byte column).
fn w_at(rule: Value, files: &[(&str, &str)]) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for (path, source) in files {
        for v in w(json!({ "ast": { "language": "python", "rule": rule.clone() } }), path, source) {
            out.push(r![path, v["span"]["startLine"].as_u64().unwrap() + 1, v["span"]["startColumn"].as_u64().unwrap()]);
        }
    }
    out
}

/// A module with nested definitions, statements side by side, and a class.
const TREE: (&str, &str) = (
    "t.py",
    "def f(a):\n    x = g(a)\n    y = h(x)\n    print(y)\n    return y\n\n\nclass C:\n    def m(self):\n        z = k(1)\n        z.w()\n        return z\n",
);

/// core.rerail's tree arithmetic, which lowering rewrites to the base relations (`rr::lower::core`), as W's matcher
/// sees it: `kind` is a node's kind; `within` W's `inside` to the end, its ancestor's kind tested (a kind set of
/// one) or read from a relation (the set of every kind); `next_sibling` W's `follows` at a neighbour (statements of a
/// block, which no token separates), negated W's `nthChild: 1`; `precedes` W's `precedes` to the end. No program
/// declares them: core.rerail's declarations are the prelude.
#[test]
fn tree_arithmetic_is_ws_relations() {
    let files = [TREE];
    let dir = corpus("tree-arithmetic", &files);
    let program = r#"
.decl at(n: node, p: symbol, l: number, c: number)
at(n, p, l, c) :- docof(n, d), doc(d, p, _), line(n, l), col(n, c).
.decl kinds(p: symbol, l: number, c: number)
kinds(p, l, c) :- kind(n, "call"), at(n, p, l, c).
.decl within_def(p: symbol, l: number, c: number)
within_def(p, l, c) :- kind(n, "identifier"), within(n, a), kind(a, "function_definition"), at(n, p, l, c).
.decl defs(a: node)
defs(a) :- kind(a, "function_definition").
.decl within_any(p: symbol, l: number, c: number)
within_any(p, l, c) :- kind(n, "identifier"), within(n, a), defs(a), at(n, p, l, c).
.decl follows_stmt(p: symbol, l: number, c: number)
follows_stmt(p, l, c) :- kind(b, "expression_statement"), next_sibling(a, b), kind(a, "expression_statement"), at(b, p, l, c).
.decl first_stmt(p: symbol, l: number, c: number)
first_stmt(p, l, c) :- kind(b, "expression_statement"), !next_sibling(_, b), at(b, p, l, c).
.decl before_return(p: symbol, l: number, c: number)
before_return(p, l, c) :- kind(a, "expression_statement"), precedes(a, b), kind(b, "return_statement"), at(a, p, l, c).
.output kinds, within_def, within_any, follows_stmt, first_stmt, before_return
"#;
    let parsed = crate::code_rule_dsl::logic::rr::parse_version(program).unwrap();
    // `within(n, a)` is the engine's tree relation (decision 0050), read as it is, not a kind set's axis.
    let within = crate::code_rule_dsl::facts::vtab::syntax::WITHIN;
    assert!(parsed.rules().flat_map(|r| r.body()).any(|l| l.atom().is_some_and(|a| parsed.rel_name(a.rel()) == within)), "{}", parsed.show());
    let got = run(&dir, program, &["kinds", "within_def", "within_any", "follows_stmt", "first_stmt", "before_return"]);
    let inside = json!({ "kind": "identifier", "inside": { "kind": "function_definition", "stopBy": "end" } });
    let want = [
        w_at(json!({ "kind": "call" }), &files),
        w_at(inside.clone(), &files),
        w_at(inside, &files),
        w_at(json!({ "kind": "expression_statement", "follows": { "kind": "expression_statement" } }), &files),
        w_at(json!({ "kind": "expression_statement", "nthChild": 1 }), &files),
        w_at(json!({ "kind": "expression_statement", "precedes": { "kind": "return_statement", "stopBy": "end" } }), &files),
    ];
    for (got, want) in got.iter().zip(want) {
        assert!(!want.is_empty());
        assert_eq!(*got, sorted(want));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// `unset(x)` has one row, no node; `docof` gives every node and span its document (each node's `node(d, …)`, each
/// line's `src_line(d, …)`), and W reports each call in the file `docof` names.
#[test]
fn docof_and_unset() {
    let files = [TREE, ("pkg/b.py", "k(1)\n")];
    let dir = corpus("docof", &files);
    let program = r#"
.decl empty(x: node)
empty(x) :- unset(x).
.decl nodes_unset(n: node)
nodes_unset(n) :- node(_, n, _, _, _), unset(n).
.decl wrong(n: node)
wrong(n) :- node(d, n, _, _, _), docof(n, e), d != e.
wrong(sp) :- src_line(d, _, sp), docof(sp, e), d != e.
.decl undocumented(n: node)
undocumented(n) :- node(_, n, _, _, _), !docof(n, _).
undocumented(sp) :- src_line(_, _, sp), !docof(sp, _).
.decl calls(p: symbol, l: number, c: number)
calls(p, l, c) :- kind(n, "call"), docof(n, d), doc(d, p, _), line(n, l), col(n, c).
.output empty, nodes_unset, wrong, undocumented, calls
"#;
    let got = run(&dir, program, &["empty", "nodes_unset", "wrong", "undocumented", "calls"]);
    assert_eq!(got[0], [["-1"]]);
    assert!(got[1..4].iter().all(|v| v.is_empty()), "{:?}", &got[1..4]);
    assert_eq!(got[4], sorted(w_at(json!({ "kind": "call" }), &files)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// `op_at`: a comparison's operators in order, two-word ones one token, as CPython's `Compare.ops` has them
/// (`a < b <= c`: Lt, LtE; `not in`: NotIn; `is not`: IsNot), and every other operator node's one. W's `has` on the
/// `operators` field sees a comparison's first operator only (so `op` has none): it finds the comparisons whose
/// `op_at(n, 0, s)` is its operator, and none for the second, `<=`.
#[test]
fn op_at_is_the_operators_in_order() {
    let files = [("o.py", "if a < b <= c:\n    pass\nz = p not in q\nw = r is not s\nv = 1 + 2 * 3\nu = not x or y\n")];
    let dir = corpus("op-at", &files);
    let program = r#"
.decl ops(k: symbol, l: number, i: number, s: symbol)
ops(k, l, i, s) :- op_at(n, i, s), kind(n, k), line(n, l).
.output ops
"#;
    let got = run(&dir, program, &["ops"]);
    // `not x` (a `not_operator`, CPython's `UnaryOp(Not)`) has its token in no field: no row.
    // (kind, line, index, operator)
    let want = [
        ("binary_operator", 5, 0, "*"),
        ("binary_operator", 5, 0, "+"),
        ("boolean_operator", 6, 0, "or"),
        ("comparison_operator", 1, 0, "<"),
        ("comparison_operator", 1, 1, "<="),
        ("comparison_operator", 3, 0, "not in"),
        ("comparison_operator", 4, 0, "is not"),
    ];
    assert_eq!(got[0], sorted(want.map(|(k, l, i, op)| r![k, l, i, op])));
    for op in ["<", "<=", "not in", "is not"] {
        let escaped = regex::escape(op);
        let at = w_at(json!({ "kind": "comparison_operator", "has": { "field": "operators", "regex": format!("^{escaped}$") } }), &files);
        let first: Vec<Vec<String>> = want.iter().filter(|w| w.0 == "comparison_operator" && w.2 == 0 && w.3 == op).map(|w| r!["o.py", w.1, if w.1 == 1 { 3 } else { 4 }]).collect();
        assert_eq!(at, first, "{op}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// `prev_token(n, s)` is the text of the child right before n among its parent's stored children, and `token(p, i, s,
/// start, end)` p's stored children in order. A named-only tape stores no token the grammar implies (RER-131), and
/// no relation holds one: the call's `(`, `)` and `,`, the `from` and the import list's `,` are in neither. A keyword
/// some other word of its gap holds is stored where implied too: `from m import (a, b)` makes `import` one.
#[test]
fn tokens_are_the_stored_neighbours() {
    let files = [("i.py", "from m import a, b\nimport x, y.z\nf(a, b, c=1)  # done\n")];
    let dir = corpus("tokens", &files);
    let program = r#"
.decl after(k: symbol, t: symbol)
after(k, t) :- node(_, n, k, _, _), prev_token(n, t).
.decl at(k: symbol, i: number, t: symbol)
at(k, i, t) :- node(_, p, k, _, _), token(p, i, t, _, _).
.output after, at
"#;
    let got = run(&dir, program, &["after", "at"]);
    let (after, at) = (&got[0], &got[1]);
    // The stored neighbours: the keyword argument follows `b`, the comment the call's statement.
    assert!(after.iter().any(|r| *r == ["keyword_argument", "b"]), "{after:?}");
    assert!(at.iter().any(|r| *r == ["argument_list", "2", "c=1"]), "{at:?}");
    assert!(at.iter().any(|r| *r == ["import_from_statement", "1", "import"]), "{at:?}");
    // No implied token, in either relation.
    for implied in ["(", ")", ",", "from"] {
        assert!(!after.iter().chain(at).any(|r| r.last().is_some_and(|v| v == implied)), "{implied}: {after:?} {at:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// `path_part(d, s)`: each directory of the path and the file name; `file_name(d, s)` the last. W's scope globs agree:
/// a file is in `**/<dir>/**` for each directory part, and in `**/<name>` for its name.
#[test]
fn path_parts_are_ws_scope() {
    let files = [("pkg/tests/test_a.py", "x = 1\n"), ("pkg/a.py", "x = 1\n"), ("b.py", "x = 1\n")];
    let dir = corpus("path-parts", &files);
    let program = r#"
.decl parts(p: symbol, s: symbol)
parts(p, s) :- doc(d, p, _), path_part(d, s).
.decl names(p: symbol, s: symbol)
names(p, s) :- doc(d, p, _), file_name(d, s).
.output parts, names
"#;
    let got = run(&dir, program, &["parts", "names"]);
    let in_scope = |glob: &str, path: &str, source: &str| {
        let capabilities: Value = serde_json::from_str(CAPABILITIES).unwrap();
        let mut document = clause_document(json!({ "id": "c", "forbid": { "ast": { "language": "python", "rule": { "kind": "module" } } }, "violation": violation() }));
        document["scope"] = json!({ "include": [glob], "exclude": [] });
        let compiled = compile_document_with(document, &capabilities).unwrap_or_else(|e| panic!("{}", e.reason));
        !evaluate_file(&compiled, "python", path, source)["violations"].as_array().unwrap().is_empty()
    };
    let (mut parts, mut names) = (Vec::new(), Vec::new());
    let candidates = ["pkg", "tests", "test_a.py", "a.py", "b.py"];
    for (path, source) in files {
        for c in candidates {
            if in_scope(&format!("**/{c}/**"), path, source) || in_scope(&format!("**/{c}"), path, source) {
                parts.push(r![path, c]);
            }
            if in_scope(&format!("**/{c}"), path, source) {
                names.push(r![path, c]);
            }
        }
    }
    assert_eq!(got, [sorted(parts), sorted(names)]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The CPython visit order's fixture: decorators, return annotation, positional and keyword-only defaults, a dict
/// with `**`, `a if t else b`, a call's keywords, a lambda's default, a class's bases and keywords.
const VISIT: &str = "@dec(a, k=1)\n@other\ndef f(p, q: int = 1, /, r=2, *args: str, s, t=3, **kw) -> R:\n    x = {k1: v1, **rest, k2: v2}\n    y = a if t else b\n    z = g(1, *xs, key=2, **more)\n    return lambda u, w=4: u\n\n\nclass C(B1, B2, metaclass=M, **opts):\n    attr: int = 5\n";

/// CPython 3.9's `ast.NodeVisitor` order of `VISIT`: each located node's byte range, first occurrences, from
/// `python3 -c` (`ast.parse`, `lineno`/`col_offset` made byte offsets).
const CPYTHON_VISIT: [(i64, i64); 62] = [
    (20, 196), (26, 27), (29, 35), (32, 35), (44, 45), (50, 59), (56, 59), (61, 62), (64, 65), (66, 67), (71, 73), (38, 39), (46, 47),
    (85, 113), (85, 86), (89, 113), (90, 92), (106, 108), (94, 96), (100, 104), (110, 112), (118, 135), (118, 119), (122, 135), (127, 128),
    (122, 123), (134, 135), (140, 168), (140, 141), (144, 168), (144, 145), (146, 147), (149, 152), (150, 152), (154, 159), (158, 159),
    (161, 167), (163, 167), (173, 196), (180, 196), (187, 188), (190, 191), (192, 193), (195, 196), (1, 12), (1, 4), (5, 6), (8, 11),
    (10, 11), (14, 19), (78, 79), (199, 254), (207, 209), (211, 213), (215, 226), (225, 226), (228, 234), (230, 234), (241, 254),
    (241, 245), (247, 250), (253, 254),
];

/// `visit_index(n, i)` orders the nodes as CPython's `ast` visits them: every node whose byte range is a located
/// `ast` node's comes in CPython's order (wrappers of the same range first), and `earlier` follows it (a decorator's
/// call after the body's return).
#[test]
fn visit_index_is_cpythons_order() {
    let dir = corpus("visit", &[("v.py", VISIT)]);
    let program = r#"
.decl visits(s: number, e: number, i: number)
visits(s, e, i) :- visit_index(n, i), node(_, n, _, s, e).
.decl later(r: number, c: number)
later(r, c) :- kind(ret, "return_statement"), kind(call, "call"), earlier(ret, call), node(_, ret, _, r, _), node(_, call, _, c, _).
.output visits, later
"#;
    let outputs = run_outputs(&dir, program);
    let mut first: std::collections::BTreeMap<(i64, i64), i64> = std::collections::BTreeMap::new();
    let mut indexes = BTreeSet::new();
    for row in outputs.rows("visits") {
        let f: Vector<i64> = row.values().map(|v| v.int().expect("a number")).collect();
        assert!(indexes.insert(f[2]), "index {} twice", f[2]);
        let at = first.entry((f[0], f[1])).or_insert(f[2]);
        *at = (*at).min(f[2]);
    }
    let cpython: Vec<(i64, i64)> = CPYTHON_VISIT.iter().copied().filter(|span| first.contains_key(span)).collect();
    assert!(cpython.len() > 50, "{} of CPython's ranges are nodes", cpython.len());
    let mut ours = cpython.clone();
    ours.sort_by_key(|span| first[span]);
    assert_eq!(ours, cpython);
    // The return statement (byte 173) comes before both decorators' calls (`dec(a, k=1)` at byte 1).
    let later = shown_of(&outputs, &["later"]).remove(0);
    assert!(later.iter().any(|l| *l == ["173", "1"]), "{later:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A disjunction over a kind variable, `(k = "call" ; k = "attribute")`, compares kinds (it was a membership test of
/// names: "$k is a Kind and a Name"), negated too: the calls and attributes W finds, and the other identifiers' parents.
#[test]
fn a_kind_disjunction_compares_kinds() {
    let files = [TREE];
    let dir = corpus("kind-disjunction", &files);
    let program = r#"
.decl either(p: symbol, l: number, c: number)
either(p, l, c) :- node(d, n, k, _, _), (k = "call" ; k = "attribute"), doc(d, p, _), line(n, l), col(n, c).
.decl neither(p: symbol, l: number, c: number)
neither(p, l, c) :- node(d, n, "identifier", _, _), child(q, n, _), node(_, q, k, _, _), !(k = "call" ; k = "attribute"),
    doc(d, p, _), line(n, l), col(n, c).
.output either, neither
"#;
    let got = run(&dir, program, &["either", "neither"]);
    let mut want = w_at(json!({ "kind": "call" }), &files);
    want.extend(w_at(json!({ "kind": "attribute" }), &files));
    assert_eq!(got[0], sorted(want));
    let others = json!({ "kind": "identifier", "inside": { "not": { "any": [{ "kind": "call" }, { "kind": "attribute" }] } } });
    assert_eq!(got[1], sorted(w_at(others, &files)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A comparison in an aggregate's body with the rule's variable, `k = min b : { …, b > o }`, reads `o` from the rule
/// (Soufflé's meaning; it was `unplaced literals`), and `count` counts the body's own variables (`_` where the
/// aggregate reads its helper): a call's first child start after its own start is its argument list's.
#[test]
fn an_aggregate_reads_the_rules_variables() {
    let dir = corpus("aggregate-outer", &[TREE]);
    let program = r#"
.decl starts(n: node, b: number)
starts(n, b) :- kind(n, "call"), child(n, c, _), node(_, c, _, b, _).
.decl after(n: node, k: number)
after(n, k) :- node(_, n, "call", o, _), k = min b : { starts(n, b), b > o }.
.decl arguments(n: node, k: number)
arguments(n, k) :- field(n, "arguments", a), node(_, a, _, k, _).
.decl later(n: node, k: number)
later(n, k) :- node(_, n, "call", o, _), k = count : { starts(n, b), b > o }.
.decl mismatch(n: node)
mismatch(n) :- after(n, k), !arguments(n, k).
mismatch(n) :- arguments(n, k), !after(n, k).
mismatch(n) :- later(n, k), k != 1.
.decl calls(n: node)
calls(n) :- later(n, _).
.output mismatch, calls
"#;
    let got = run(&dir, program, &["mismatch", "calls"]);
    assert!(got[0].is_empty(), "{:?}", got[0]);
    assert_eq!(got[1].len(), 5, "{:?}", got[1]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A recursive relation with facts whose every rule joins an empty relation of a lower stratum keeps its facts: the
/// stratum has no rule to run, but its relations are not held empty (`native_stratum_`; the data-layer pack's
/// `conditional_derived`, a fact and a rule over the classes' bases, where no class has one).
#[test]
fn a_stratum_whose_rules_join_nothing_keeps_its_facts() {
    let dir = corpus("facts-empty-joins", &[TREE]);
    let program = ".decl s(x: number)\ns(n) :- node(_, _, k, n, _), k = \"no_such_kind\".\n\
                   .decl e(x: number, y: number)\ne(x, y) :- s(x), s(y).\n\
                   .decl d(x: number)\nd(7).\nd(y) :- d(x), e(x, y).\n\
                   .decl out(x: number)\nout(x) :- d(x).\n.output out\n";
    assert_eq!(run(&dir, program, &["out"]), [[["7"]]]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A rule that copies a bound input's columns (`kind(n, k) :- node(_, n, k, _, _)`, the data-layer prelude's) is
/// held as a view of the input, not derived by a stratum (Q37); its readers find what the derived copy gave them.
/// A rule that drops the input's key, or filters it, is still a stratum.
#[test]
fn a_copy_of_an_inputs_columns_is_a_view_of_it() {
    let dir = corpus("input-view", &[TREE]);
    let program = ".decl kind(n: node, k: symbol)\nkind(n, k) :- node(_, n, k, _, _).\n\
                   .decl kinds(k: symbol)\nkinds(k) :- node(_, _, k, _, _).\n\
                   .decl calls(s: number, k: symbol)\ncalls(s, k) :- kind(n, \"call\"), node(_, n, _, s, _), kinds(k), k = \"call\".\n\
                   .output calls\n";
    // Each run's own scope, held here: the views it held are the run's count (decision 0442), read before it ends.
    let scope = || crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
    let viewed = {
        let _run = scope();
        let viewed = run(&dir, program, &["calls"]);
        let held = crate::code_rule_dsl::logic::plan::input_views();
        assert!(held >= 1, "kind is a view of node: {held}");
        viewed
    };
    let copied = {
        let _copy = super::switches::set(&[("RERAIL_COPY_VIEWS", "1")]);
        let _run = scope();
        std::fs::remove_dir_all(dir.join(".out")).ok();
        let got = run(&dir, program, &["calls"]);
        assert_eq!(crate::code_rule_dsl::logic::plan::input_views(), 0, "RERAIL_COPY_VIEWS derives it");
        got
    };
    assert_eq!(viewed, copied);
    assert_eq!(viewed[0].len(), 5, "the five calls: {viewed:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A copy of any input's columns, not only `node`'s, is a view held in place (decisions 0329, 0353): no row of it is
/// read or copied when it is held. Read into flat rows, a view of a batch's `string_annotation` made every document
/// of a warm run that read none of it (ep2's `rr_ctx29`, a third of the run, Q332). Its readers find what the
/// derived copy gave them.
#[test]
fn a_copy_of_any_inputs_columns_is_held_in_place() {
    use crate::code_rule_dsl::logic::profile;
    let dir = corpus("input-view-any", &[TREE]);
    let program = ".decl under(x: node, y: node)\nunder(x, y) :- within(x, y).\n\
                   .decl in_def(s: number)\nin_def(s) :- under(n, a), node(_, a, \"function_definition\", _, _), node(_, n, \"identifier\", s, _).\n\
                   .output in_def\n";
    let scope = || crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
    let (viewed, held) = {
        let _run = scope();
        profile::set(true);
        profile::take_decisions();
        let viewed = run(&dir, program, &["in_def"]);
        let made = profile::take_decisions();
        profile::set(false);
        let held: BTreeSet<String> = made.into_keys().filter(|(t, _)| t == "a view held in place: its statistics (Q332)").map(|(_, x)| x).collect();
        (viewed, held)
    };
    assert!(held.iter().any(|x| x.starts_with("under: ")), "under is a view of within, held in place: {held:?}");
    let copied = {
        let _copy = super::switches::set(&[("RERAIL_COPY_VIEWS", "1")]);
        let _run = scope();
        std::fs::remove_dir_all(dir.join(".out")).ok();
        run(&dir, program, &["in_def"])
    };
    assert_eq!(viewed, copied);
    assert!(!viewed[0].is_empty(), "the identifiers inside a function: {viewed:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A core relation nothing derives and the runner does not bind is a named error, not the planner's missing
/// statistics; those that need CPython say so. A program's own input is its business: no such error.
#[test]
fn an_unbound_relation_is_named() {
    let dir = corpus("unbound", &[TREE]);
    // Each run its own output directory: a run that fails after opening its store leaves it half made.
    let fails = |out: &str, program: &str| {
        super::run(program, &dir, None, &dir.join(out), "program", Binding::Cpu, 1 << 18).err().map(|e| e.to_string()).unwrap_or_default()
    };
    // `git_tracked` is a functor now (`@git_tracked`); `signed_list` is still a core.rerail input no producer binds (O80).
    let e = fails(".out-signed", ".decl v(p: symbol, e: symbol)\nv(p, e) :- signed_list(p, e).\n.output v\n");
    assert!(e.contains("signed_list is read, and nothing derives it or binds it"), "{e}");
    let e = fails(".out-str", ".decl v(n: node)\nv(n) :- str_part(n, 0, _, _, _).\n.output v\n");
    assert!(e.contains("str_part is read") && e.contains("CPython"), "{e}");
    let e = fails(".out-mine", ".decl mine(n: node)\n.input mine\n.decl v(n: node)\nv(n) :- mine(n).\n.output v\n");
    assert!(!e.contains("nothing derives it or binds it"), "{e}");
    eprintln!("UNBOUND mine: {e:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The run that failed before (`an_unbound_relation_is_named`'s fourth case in the first three's directory): a
/// failed run's store reopened.
#[test]
#[ignore = "a report: a run reusing a failed run's output directory"]
fn a_failed_runs_store_reopened_report() {
    let dir = corpus("unbound-shared", &[TREE]);
    let out = dir.join(".out");
    for program in [".decl v(n: node)\nv(n) :- str_part(n, 0, _, _, _).\n.output v\n", ".decl mine(n: node)\n.input mine\n.decl v(n: node)\nv(n) :- mine(n).\n.output v\n"] {
        let e = super::run(program, &dir, None, &out, "program", Binding::Cpu, 1 << 18).err().map(|e| e.to_string());
        eprintln!("SHARED {e:?}; left in .out: {:?}", std::fs::read_dir(&out).map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name()).collect::<Vec<_>>()));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// `as` names a variable (python/calls.rerail's `field(c, "arguments", as)`), as the engine always read it. Soufflé
/// reserves it (its `as(x, T)` cast), so a program for Soufflé renames it.
#[test]
fn as_is_a_variable() {
    let files = [TREE];
    let dir = corpus("as-variable", &files);
    let program = r#"
.decl args(p: symbol, l: number, c: number)
args(p, l, c) :- kind(call, "call"), field(call, "arguments", as), kind(as, "argument_list"), docof(as, d), doc(d, p, _), line(as, l), col(as, c).
.output args
"#;
    assert_eq!(run(&dir, program, &["args"])[0], sorted(w_at(json!({ "kind": "argument_list", "inside": { "kind": "call" } }), &files)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// F-strings W's `pythonFString` reads: escapes, a raw prefix with `{{`, a debug field, an f-string beside a plain
/// literal (both orders), a triple-quoted multiline one with a conversion and a nested spec field, a spec, a string
/// nested in a spec, `\N{…}`; and what has no tree: plain literals, bytes.
const FSTRINGS: &str = r#"x = y = w = 1
a = f'\x53ELECT {x} FROM t'
b = rf'SELECT {{name}} {x} FROM t'
c = f'{SELECT = } FROM t'
d = (f'SELECT {x}' ' FROM t')
e = f"""SELECT {x}
  FROM t
 WHERE {y!r:>{w}} = 1"""
g = f'a{x!r:>10}b{y=}'
h = 'plain' "also"
i = b'bytes'
j = f'{x:{"nested"}}'
k = f"é{x}\N{BULLET}"
l = 'plain' f'{x}'
"#;

/// W's `parse: {grammar: pythonFString, …}` at its hosts, `inner` the parse's rule: the spans it finds, each anchored
/// at `anchor` (the host, or `capture.NAME`).
fn w_fstring(inner: Value, anchor: &str) -> BTreeSet<(usize, usize)> {
    let hosts = json!({ "any": [{ "kind": "string" }, { "kind": "concatenated_string" }, { "kind": "format_specifier" }] });
    let mut parse = json!({ "grammar": "pythonFString" });
    parse.as_object_mut().unwrap().extend(inner.as_object().unwrap().clone());
    let forbid = json!({ "ast": { "language": "python", "rule": { "all": [hosts, { "parse": parse }] } } });
    let v = if anchor == "host" { violation() } else { json!({ "anchor": anchor, "code": "X", "message": "x" }) };
    w_reporting(forbid, v, "python", "a.py", FSTRINGS)
        .iter()
        .map(|f| (f["span"]["startByte"].as_u64().unwrap() as usize, f["span"]["endByte"].as_u64().unwrap() as usize))
        .collect()
}

/// `fstring_template(n, marker, v)` is the root value of W's `parse: {grammar: pythonFString, interpolation: marker}`
/// and `fstring_piece(n, i, kind, v)` its root's children in order, at exactly the hosts W gets a tree at.
#[test]
fn fstrings_are_ws_python_fstring_trees() {
    let dir = corpus("fstring", &[("a.py", FSTRINGS)]);
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl fstring_piece(n: node, i: number, kind: symbol, v: symbol)
.decl fstring_template(n: node, marker: symbol, v: symbol)
.decl pieces(s: number, e: number, i: number, k: symbol, v: symbol)
pieces(s, e, i, k, v) :- fstring_piece(n, i, k, v), node(_, n, _, s, e).
.decl dots(s: number, e: number, v: symbol)
dots(s, e, v) :- fstring_template(n, "{...}", v), node(_, n, _, s, e).
.decl braces(s: number, e: number, v: symbol)
braces(s, e, v) :- fstring_template(n, "{}", v), node(_, n, _, s, e).
.decl from_t(s: number, e: number)
from_t(s, e) :- fstring_template(n, "{...}", v), match("(?s:.*)(?:SELECT [{][.][.][.][}] FROM t)(?s:.*)", v), node(_, n, _, s, e).
.output pieces, dots, braces, from_t
"#;
    let outputs = run_outputs(&dir, program);
    let number = |v: crate::code_rule_dsl::logic::plan::output::Value<'_>| usize::try_from(v.int().expect("a number")).unwrap();
    // A relation's rows of a span (two numbers) and a symbol.
    let spanned = |rel: &str| -> Vec<((usize, usize), String)> {
        outputs.rows(rel).iter().map(|r| ((number(r.get(0)), number(r.get(1))), r.get(2).text().expect("a symbol").to_owned())).collect()
    };
    let hosts = w_fstring(json!({ "kind": "template" }), "host");
    for (rel, marker) in [("dots", "{...}"), ("braces", "{}")] {
        let rows = spanned(rel);
        assert_eq!(rows.iter().map(|r| r.0).collect::<BTreeSet<_>>(), hosts, "{marker}: {rows:?}");
        for (span, v) in &rows {
            let at = w_fstring(json!({ "interpolation": marker, "value": { "eq": v } }), "host");
            assert!(at.contains(span), "{marker}: {v:?} is not W's root value at {:?}", &FSTRINGS[span.0..span.1]);
        }
    }
    let dots: Vec<(String, String)> = spanned("dots").into_iter().map(|(span, v)| (FSTRINGS[span.0..span.1].to_owned(), v)).collect();
    let template = |source: &str| dots.iter().filter(|(s, _)| s == source).map(|(_, v)| v.as_str()).collect::<Vec<_>>();
    assert_eq!(template(r"f'\x53ELECT {x} FROM t'"), ["SELECT {...} FROM t"]);
    assert_eq!(template("rf'SELECT {{name}} {x} FROM t'"), ["SELECT {name} {...} FROM t"]);
    assert_eq!(template("f'{SELECT = } FROM t'"), ["SELECT = {...} FROM t"]);
    assert_eq!(template("(f'SELECT {x}' ' FROM t')"), Vec::<&str>::new(), "the parenthesized expression is no host");
    assert_eq!(template("f'SELECT {x}' ' FROM t'"), ["SELECT {...} FROM t"]);
    assert_eq!(template("f'SELECT {x}'"), ["SELECT {...}"]);
    assert_eq!(template("f\"\"\"SELECT {x}\n  FROM t\n WHERE {y!r:>{w}} = 1\"\"\""), ["SELECT {...}\n  FROM t\n WHERE {...} = 1"]);
    assert_eq!(template(":>{w}"), [">{...}"]);
    assert_eq!(template("f'a{x!r:>10}b{y=}'"), ["a{...}by={...}"]);
    assert_eq!(template("'plain' f'{x}'"), ["plain{...}"]);
    for none in ["'plain' \"also\"", "'plain'", "b'bytes'", "\"nested\"", "' FROM t'"] {
        assert_eq!(template(none), Vec::<&str>::new(), "{none}");
    }
    // The text test over the template reads the batch's strings.
    let from_t: BTreeSet<&str> = outputs.rows("from_t").iter().map(|r| &FSTRINGS[number(r.get(0))..number(r.get(1))]).collect();
    assert_eq!(from_t, BTreeSet::from([r"f'\x53ELECT {x} FROM t'", "f'SELECT {x}' ' FROM t'"]));

    // Pieces: at each host W has a tree at, W's root child with i siblings before it (W has no `nthChild` in a parse)
    // is the row's kind (a literal with the row's value, an interpolation standing for the field the row's expression
    // opens), and no child has more siblings before it than the last row's i.
    // Each piece: its host's span, its index, its kind and its value.
    let pieces: Vec<((usize, usize), usize, String, String)> = outputs
        .rows("pieces")
        .iter()
        .map(|r| ((number(r.get(0)), number(r.get(1))), number(r.get(2)), r.get(3).text().expect("a kind").to_owned(), r.get(4).text().expect("a value").to_owned()))
        .collect();
    let mut last: std::collections::BTreeMap<(usize, usize), usize> = Default::default();
    for (span, i, kind, v) in &pieces {
        let ((s, e), i) = (*span, *i);
        let (kind, v) = (kind.as_str(), v.as_str());
        let host = &FSTRINGS[s..e];
        let at_most = last.entry((s, e)).or_default();
        *at_most = (*at_most).max(i);
        let nth = json!({ "count": { "follows": { "stopBy": "end", "any": [{ "kind": "literal" }, { "kind": "interpolation" }] }, "min": i, "max": i } });
        match kind {
            "literal" => {
                let at = w_fstring(json!({ "has": { "all": [{ "kind": "literal" }, nth, { "value": { "eq": v } }] } }), "host");
                assert!(at.contains(&(s, e)), "{host}: child {i} is not W's literal {v:?}");
            }
            "interpolation" => {
                let fields = w_fstring(json!({ "has": { "all": [{ "kind": "interpolation" }, nth], "capture": "I" } }), "capture.I");
                let field = fields.iter().filter(|(fs, fe)| s <= *fs && *fe <= e).map(|&(fs, fe)| &FSTRINGS[fs..fe]).find(|f| f.starts_with(&format!("{{{v}")));
                assert!(field.is_some(), "{host}: child {i} is no W interpolation over a field of {v:?} ({fields:?})");
            }
            "debug" => {
                let before = pieces.iter().find(|r| r.0 == (s, e) && r.1 == i - 1 && r.2 == "literal").unwrap();
                assert!(before.3.ends_with(v), "{host}: W's literal before child {i} holds the debug text {v:?}");
            }
            "format_spec" => {
                let interpolation = pieces.iter().find(|r| r.0 == (s, e) && r.1 == i && r.2 == "interpolation").unwrap();
                assert!(host.contains(&format!("{{{}", interpolation.3)) && host.contains(&format!(":{v}}}")), "{host}: spec {v:?}");
            }
            other => panic!("kind {other}"),
        }
    }
    assert_eq!(last.keys().copied().collect::<BTreeSet<_>>(), hosts);
    for (&(s, e), &i) in &last {
        let later = json!({ "count": { "follows": { "stopBy": "end", "any": [{ "kind": "literal" }, { "kind": "interpolation" }] }, "min": i + 1 } });
        let past = w_fstring(json!({ "has": later }), "host");
        assert!(!past.contains(&(s, e)), "{}: W's root has a child past {i}", &FSTRINGS[s..e]);
    }
    // A host's pieces, each as "index kind value", in order.
    let of = |source: &str| {
        let mut of = pieces.iter().filter(|r| &FSTRINGS[r.0.0..r.0.1] == source).map(|r| (r.1, r.2.as_str(), r.3.as_str())).collect::<Vec<_>>();
        of.sort();
        of.into_iter().map(|(i, k, v)| format!("{i} {k} {v}")).collect::<Vec<_>>()
    };
    assert_eq!(of("f'a{x!r:>10}b{y=}'"), ["0 literal a", "1 format_spec >10", "1 interpolation x", "2 literal by=", "3 debug y=", "3 interpolation y"]);
    assert_eq!(of("f'{SELECT = } FROM t'"), ["0 literal SELECT = ", "1 debug SELECT = ", "1 interpolation SELECT", "2 literal  FROM t"]);
    assert_eq!(of(":>{w}"), ["0 literal >", "1 interpolation w"]);
    assert_eq!(of("f'{x:{\"nested\"}}'"), ["0 format_spec {\"nested\"}", "0 interpolation x"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// internal-dead-code's decorator names: a decorator's line (`src_line`) and column, the `match_group` whose match `m`
/// is at that column (`col(m, c)` joined on a bound `c`), and the group's text. W: the `lines` regex's capture at
/// each decorator's line and column.
#[test]
fn match_group_joined_on_its_match_column() {
    let source = "@app.route('/x')\ndef f(): pass\n\nclass C:\n    @ staticmethod\n    def g(): pass\n    x = '@notadeco'\n";
    let dir = corpus("match-group-col", &[("a.py", source)]);
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl field(p: node, f: symbol, c: node)
.decl child(p: node, c: node, i: number)
.decl src_line(d: number, l: number, sp: span)
.decl text(n: loc, s: symbol)
.decl line(n: loc, l: number)
.decl col(n: loc, c: number)
.decl deco_at(o: node, d: number, l: number, c: number)
deco_at(o, d, l, c) :- node(d, dd, "decorated_definition", _, _), field(dd, "definition", o), child(dd, dc, _), node(_, dc, "decorator", _, _), line(dc, l), col(dc, c).
.decl deco(l: number, x: symbol)
deco(l, x) :- deco_at(_, d, l, c), src_line(d, l, sp), match_all("@\\s*(?P<n>[^\\s(]+)", sp, m), match_group("@\\s*(?P<n>[^\\s(]+)", sp, m, "n", g), col(m, c), text(g, x).
.output deco
"#;
    let got = run(&dir, program, &["deco"]);
    let decorators: BTreeSet<(u64, u64)> = w(json!({ "ast": { "language": "python", "rule": { "kind": "decorator" } } }), "a.py", source)
        .iter()
        .map(|v| (v["span"]["startLine"].as_u64().unwrap(), v["span"]["startColumn"].as_u64().unwrap()))
        .collect();
    let regex = json!({ "regex": { "source": "file.content", "pattern": "@\\s*(?P<n>[^\\s(]+)", "lines": {} } });
    let at = json!({ "anchor": "capture.n", "code": "X", "message": "x" });
    let whole = w_as(regex.clone(), "python", "a.py", source);
    let groups = w_reporting(regex, at, "python", "a.py", source);
    let want = sorted(whole.iter().zip(&groups).filter(|(m, _)| decorators.contains(&(m["span"]["startLine"].as_u64().unwrap(), m["span"]["startColumn"].as_u64().unwrap()))).map(|(_, g)| {
        let at = |k: &str| g["span"][k].as_u64().unwrap() as usize;
        r![at("startLine") + 1, &source[at("startByte")..at("endByte")]]
    }));
    assert_eq!(want, [["1", "app.route"], ["5", "staticmethod"]]);
    assert_eq!(got[0], want);
    let _ = std::fs::remove_dir_all(&dir);
}

/// connection-structural-boundaries' first and last classes: `min`/`max` over an input atom with a constant column
/// (`node(d, _, "function_definition", t, _)`). The input is read without its constant columns, and the aggregated
/// column is its place among those read (the last one was read past the row).
#[test]
fn min_and_max_over_an_input_with_a_constant_column() {
    let source = "x = 1\ndef f(): pass\ndef g():\n    return 2\n";
    let dir = corpus("min-max-constant-column", &[("a.py", source)]);
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl first_def(s: number)
first_def(s) :- node(d, _, "module", _, _), s = min t : { node(d, _, "function_definition", t, _) }.
.decl last_end(e: number)
last_end(e) :- node(d, _, "module", _, _), e = max t : { node(d, _, "function_definition", _, t) }.
.output first_def, last_end
"#;
    let got = run(&dir, program, &["first_def", "last_end"]);
    assert_eq!(got, [[[source.find("def f").unwrap().to_string()]], [[(source.find("2\n").unwrap() + 1).to_string()]]]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// serverplan-target-architecture's required shapes: a demanded relation (decision 0048) read with constants alone
/// (`!ident("g")`). The reader passes no variable on, so its `pre_<i>` is nullary, its body empty or a negation
/// alone, and takes the presence column the physical form gives every nullary relation.
#[test]
fn a_demanded_relation_read_with_constants_alone() {
    let source = "f = 1\nh = f\n";
    let dir = corpus("demand-constants-alone", &[("a.py", source)]);
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl text(n: loc, s: symbol)
.decl ident(s: symbol)
ident(s) :- node(_, n, "identifier", _, _), text(n, s).
.decl other(s: symbol)
other("f").
.decl missing(s: symbol)
missing("f") :- !ident("f").
missing("g") :- !ident("g").
missing("h") :- !other("h"), !ident("h").
missing("x") :- !other("x"), !ident("x").
.output missing
"#;
    let got = run(&dir, program, &["missing"]);
    assert_eq!(got, [[["g"], ["x"]]]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// internal-dead-code's dynamic imports: two groups of one match (`match_group` twice on `m`), the match's `line` and
/// `col`, and a disjunction over them (one rule per alternative, each reading the match relations).
#[test]
fn match_groups_of_one_match_and_a_disjunction() {
    let re = r#"(?:\bimport_module|\bimportlib\.import_module)\(\s*['"](?P<module>[^'"]+)['"]\s*\)\.(?P<attr>[A-Za-z_][A-Za-z0-9_]*)"#;
    let source = "import importlib\nx = importlib.import_module('pkg.mod').run\n  y = import_module( \"a.b\" ).c\n";
    let dir = corpus("match-groups-disjunction", &[("a.py", source)]);
    let quoted = re.replace('\\', "\\\\").replace('"', "\\\"");
    let program = r#"
.decl source(d: number, sp: span)
.decl text(n: loc, s: symbol)
.decl line(n: loc, l: number)
.decl col(n: loc, c: number)
.decl dyn_at(fq: symbol, l: number, c: number)
dyn_at(fq, l, c) :- source(_, sp), match_group("RE", sp, m, "module", gm), match_group("RE", sp, m, "attr", ga), line(m, l), col(m, c), text(gm, mn), text(ga, an), fq = cat(mn, cat(".", an)).
.decl late(fq: symbol)
late(fq) :- source(_, sp), match_all("RE", sp, m), match_group("RE", sp, m, "module", gm), text(gm, fq), (line(m, l), l > 2 ; col(m, c), c < 5).
.output dyn_at, late
"#
    .replace("RE", &quoted);
    let got = run(&dir, &program, &["dyn_at", "late"]);
    let regex = json!({ "regex": { "source": "file.content", "pattern": re } });
    let whole = w_as(regex.clone(), "python", "a.py", source);
    let group = |g: &str| w_reporting(regex.clone(), json!({ "anchor": format!("capture.{g}"), "code": "X", "message": "x" }), "python", "a.py", source);
    let text = |v: &Value| &source[v["span"]["startByte"].as_u64().unwrap() as usize..v["span"]["endByte"].as_u64().unwrap() as usize];
    let (modules, attrs) = (group("module"), group("attr"));
    let want = sorted(whole.iter().zip(modules.iter().zip(&attrs)).map(|(m, (gm, ga))| {
        r![format!("{}.{}", text(gm), text(ga)), m["span"]["startLine"].as_u64().unwrap() + 1, m["span"]["startColumn"]]
    }));
    assert_eq!(want, [["a.b.c", "3", "6"], ["pkg.mod.run", "2", "4"]]);
    assert_eq!(got[0], want);
    assert_eq!(got[1], [["a.b"], ["pkg.mod"]]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// core.rerail's `syntax_error(d, l, msg)` is CPython's verdict (`cpython::syntax`), bound as `cpython_syntax_error`: a file
/// tree-sitter reads without an error node (an empty block) is rejected, as is an invalid target, with CPython's line and
/// message; one only `compile` rejects (`return` outside a function) is not. A program that declares
/// `syntax_error(d: number)` reads the tape's verdict, and both may be read side by side.
#[test]
fn syntax_error_is_cpythons_verdict() {
    let files = [
        ("empty.py", "x = 1\nif x:\n# nothing\n"),
        ("target.py", "f() = 1\n"),
        ("ret.py", "return 1\n"),
        ("bad.py", "def f(:\n    pass\n"),
        ("ok.py", "x = 1\n"),
    ];
    let dir = corpus("cpython-syntax", &files);
    let core = r#"
.decl doc(d: number, path: symbol, module: symbol)
.decl rejected(p: symbol, l: number, msg: symbol)
rejected(p, l, msg) :- doc(d, p, _), syntax_error(d, l, msg).
.decl invalid(p: symbol)
invalid(p) :- doc(d, p, _), syntax_error(d, _, m), prefix("invalid", m).
.output rejected, invalid
"#;
    let got = run(&dir, core, &["rejected", "invalid"]);
    assert_eq!(
        got[0],
        [["bad.py", "1", "invalid syntax"], ["empty.py", "3", "expected an indented block after 'if' statement on line 2"], ["target.py", "1", "cannot assign to function call here. Maybe you meant '==' instead of '='?"]]
    );
    assert_eq!(got[1], [["bad.py"]]);
    let both = r#"
.decl doc(d: number, path: symbol, module: symbol)
.decl syntax_error(d: number)
.decl cpython_syntax_error(d: number, l: number, msg: symbol)
.decl only_cpython(p: symbol)
only_cpython(p) :- doc(d, p, _), cpython_syntax_error(d, _, _), !syntax_error(d).
.decl only_tree(p: symbol)
only_tree(p) :- doc(d, p, _), syntax_error(d), !cpython_syntax_error(d, _, _).
.output only_cpython, only_tree
"#;
    let got = run(&dir, both, &["only_cpython", "only_tree"]);
    // tree-sitter has an error node at `f() = 1` too.
    assert_eq!(got[0], [["empty.py"]]);
    assert!(got[1].is_empty(), "{:?}", got[1]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `string_annotation(s)` is each string Ruff's semantic model reads as a type definition (the scope table's), nested
/// ones included, and `parsed(s, "annotation", root)` its tree, a tape value asked for by `string_annotation`: the
/// value parsed as the `annotation` fragment, from a `type` (decision 0466), its identifiers `text` rows as a file's
/// are; a nested string's value is made in the next round. A value the parser refuses (`"not valid (("`, which Ruff
/// refuses too) is one `ERROR` over the text, with no names. A `Literal` argument is no type definition.
#[test]
fn string_annotations_are_ruffs_with_their_trees() {
    let source = "from typing import List, Literal, Optional\nimport typing\n\nx: \"List[int]\" = []\n\ndef f(a: \"Optional[Foo]\", b: 'typing.Dict[str, \"Bar\"]') -> \"Baz\":\n    pass\n\ny: \"not valid ((\" = 1\nz: Literal[\"notatype\"] = 1\n";
    let dir = corpus("string-annotations", &[("a.py", source)]);
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl text(n: node, s: symbol)
.decl node_text(n: node, s: symbol)
.decl annotated(t: symbol)
annotated(t) :- string_annotation(n), node_text(n, t).
.decl names(t: symbol, x: symbol)
names(t, x) :- string_annotation(n), parsed(n, "annotation", r), node_text(n, t), within(m, r), node(_, m, "identifier", _, _), text(m, x).
.decl roots(t: symbol, k: symbol, s: number, e: number)
roots(t, k, s, e) :- string_annotation(n), parsed(n, "annotation", r), node_text(n, t), node(_, r, k, s, e).
.decl files(p: symbol)
files(p) :- doc(_, p, _).
.output annotated, names, roots, files
"#;
    let got = run(&dir, program, &["annotated", "names", "roots", "files"]);
    let set = |rows: &[Vec<String>]| rows.iter().cloned().collect::<BTreeSet<Vec<String>>>();
    let want = |rows: &[&[&str]]| rows.iter().map(|r| r.iter().map(|v| (*v).to_owned()).collect()).collect::<BTreeSet<Vec<String>>>();
    assert_eq!(
        set(&got[0]),
        want(&[&["\"List[int]\""], &["\"Optional[Foo]\""], &["'typing.Dict[str, \"Bar\"]'"], &["\"Bar\""], &["\"Baz\""], &["\"not valid ((\""]])
    );
    assert_eq!(
        set(&got[1]),
        want(&[
            &["\"List[int]\"", "List"],
            &["\"List[int]\"", "int"],
            &["\"Optional[Foo]\"", "Optional"],
            &["\"Optional[Foo]\"", "Foo"],
            &["'typing.Dict[str, \"Bar\"]'", "typing"],
            &["'typing.Dict[str, \"Bar\"]'", "Dict"],
            &["'typing.Dict[str, \"Bar\"]'", "str"],
            &["\"Bar\"", "Bar"],
            &["\"Baz\"", "Baz"],
        ])
    );
    // Each root is its value's `type`, over the string's value (decision 0466); a refused one's an `ERROR`.
    assert_eq!(
        set(&got[2]),
        want(&[
            &["\"List[int]\"", "type", "0", "9"],
            &["\"Optional[Foo]\"", "type", "0", "13"],
            &["'typing.Dict[str, \"Bar\"]'", "type", "0", "23"],
            &["\"Bar\"", "type", "0", "3"],
            &["\"Baz\"", "type", "0", "3"],
            &["\"not valid ((\"", "ERROR", "0", "12"],
        ])
    );
    // The trees are no files.
    assert_eq!(got[3], [["a.py"]]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `literal_json(n, v)` is W's pythonAst `literal_json`: CPython's `json.dumps(ast.literal_eval(node), sort_keys=True,
/// separators=(",", ":"))` at the nodes W's worker reads as the `ast` node of their span (a `type` node too), none
/// where it raises; an implicitly concatenated string's parts are no `ast` node.
#[test]
fn literal_json_is_ws_python_ast_field() {
    let source = "x = {'b': 1, 'a': (1, 2.5)}\ny = -1\nz = [f(), 1e16]\nw = 'é' 'x'\na: [None] = {1, 2}\nb = {'k': b'v'}\n";
    let dir = corpus("literal-json", &[("a.py", source)]);
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl node_text(n: node, s: symbol)
.decl vals(k: symbol, t: symbol, v: symbol)
vals(k, t, v) :- literal_json(n, v), node(_, n, k, _, _), node_text(n, t).
.output vals
"#;
    let got = run(&dir, program, &["vals"]);
    assert_eq!(
        got[0],
        sorted([
            ["dictionary", "{'b': 1, 'a': (1, 2.5)}", "{\"a\":[1,2.5],\"b\":1}"],
            ["float", "1e16", "1e+16"],
            ["float", "2.5", "2.5"],
            ["integer", "1", "1"],
            ["integer", "1", "1"],
            ["integer", "1", "1"],
            ["integer", "2", "2"],
            ["none", "None", "null"],
            ["concatenated_string", "'é' 'x'", "\"\\u00e9x\""],
            ["string", "'a'", "\"a\""],
            ["string", "'b'", "\"b\""],
            ["string", "'k'", "\"k\""],
            ["tuple", "(1, 2.5)", "[1,2.5]"],
            ["type", "[None]", "[null]"],
            ["list", "[None]", "[null]"],
            ["unary_operator", "-1", "-1"],
        ]
        .map(|r| r.map(str::to_owned).to_vec()))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `@py_unparse(n)` is CPython 3.13's `ast.unparse` of the `ast` node at n's span that W's worker reads n's kind as,
/// and `@py_repr(@py_unparse(n))` W's `repr_unparse`. The values are CPython 3.13.14's.
#[test]
fn unparse_is_ws_python_ast_field() {
    let source = "def f(a: \"x\", *b: int, c=1) -> dict[str, int]:\n    return a\nx = (a or b) and not c\ny = f\"{x!r:>{w}} {y=}\"\n";
    let dir = corpus("unparse", &[("a.py", source)]);
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl node_text(n: node, s: symbol)
.decl printed(k: symbol, t: symbol, u: symbol, r: symbol)
printed(k, t, @py_unparse(n), @py_repr(@py_unparse(n))) :- node(_, n, k, _, _), node_text(n, t), k != "identifier", k != "expression_statement".
.output printed
"#;
    let got = run(&dir, program, &["printed"]);
    let has = |row: &[&str]| got[0].iter().any(|l| l[..] == *row);
    for row in [
        ["type", "\"x\"", "'x'", "\"'x'\""],
        ["type", "int", "int", "'int'"],
        ["type", "str", "str", "'str'"],
        ["type", "dict[str, int]", "dict[str, int]", "'dict[str, int]'"],
        ["boolean_operator", "(a or b) and not c", "(a or b) and (not c)", "'(a or b) and (not c)'"],
        ["boolean_operator", "a or b", "a or b", "'a or b'"],
        ["not_operator", "not c", "not c", "'not c'"],
        ["string", "f\"{x!r:>{w}} {y=}\"", "f'{x!r:>{w}} y={y!r}'", "\"f'{x!r:>{w}} y={y!r}'\""],
        ["integer", "1", "1", "'1'"],
        ["typed_parameter", "a: \"x\"", "a: 'x'", "\"a: 'x'\""],
    ] {
        assert!(has(&row), "{row:?} in {:?}", got[0]);
    }
    // CPython's `arg` of `*b: int` starts at `b`: tree-sitter's parameter node, with its `*`, is no `arg`'s span.
    assert!(!got[0].iter().any(|l| l[0] == "typed_parameter" && l[1].starts_with("*b")), "{:?}", got[0]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `@sha256(@ast_dump(n))` is W's `dump_sha256`, which `dump_sha256(n, h)` gives; `body_sha256` and `signature_sha256`
/// W's hashes of a def. The values are CPython 3.13.14's.
#[test]
fn dumps_are_ws_python_ast_fields() {
    let source = "def f(a: \"x\", *b: int, c=1) -> dict[str, int]:\n    return a\nx = f(1)\n";
    let dir = corpus("dumps", &[("a.py", source)]);
    let program = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl defs(b: symbol, s: symbol, d: symbol)
defs(b, s, d) :- body_sha256(f, b), signature_sha256(f, s), d = @sha256(@ast_dump(f)), dump_sha256(f, d).
.decl calls(d: symbol)
calls(@ast_dump(n)) :- node(_, n, "call", _, _).
.output defs, calls
"#;
    let got = run(&dir, program, &["defs", "calls"]);
    assert_eq!(
        got[0],
        [["747bc3749caaa6d222424d1542da2ee0a312fe508fb3081c65d873c88311bae1", "f162c45775dc52a80bbf80593192690791f38ed2ee5db47bdd0582cf37943108", "86250c1f242392eacdb303a6c1df981f77a09a769fdd196f207f0e2ec43ae458"]]
    );
    assert_eq!(got[1], [["Call(func=Name(id='f', ctx=Load()), args=[Constant(value=1)])"]]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A text test over `node_text` answered on request (decision 0174): `contains` reads the answer's text.
#[test]
fn a_text_test_reads_node_text_answered_on_request() {
    let dir = corpus("node-text-contains", &[("a.py", "class SqlAdminQuery:\n    pass\n"), ("b.py", "x = 1\n")]);
    let program = r#"
.decl node(d: node, n: node, k: symbol, s: number, e: number)
.decl doc(d: node, p: symbol, m: symbol)
.decl node_text(n: node, s: symbol)
.input node, doc, node_text
.decl gate(p: symbol)
gate(p) :- node(d, m, "module", _, _), doc(d, p, _), node_text(m, s), contains("SqlAdminQuery", s).
.decl pre(p: symbol)
pre(p) :- node(d, m, "module", _, _), doc(d, p, _), node_text(m, s), prefix("class", s).
.output gate, pre
"#;
    let got = run(&dir, program, &["gate", "pre"]);
    assert_eq!(got, [[["a.py"]], [["a.py"]]]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `str` (a string literal's content) is answered on request (decisions 0174, 0353 step 3): a reader that binds the
/// node by its kind asks for those nodes only, and gets the rows the eager relation has. The oracle is a reader that
/// binds the node by nothing, which keeps `str` an input made for every node. Python's prefixes, bytes, raw, f-strings
/// with and without interpolation, a concatenation's parts, a docstring.
#[test]
fn str_answered_on_request_holds_the_eager_rows() {
    let dir = corpus("str-on-request", &[("a.py", "x = 'a'\ny = b\"by\"\nz = r'\\\\d'\nw = f'k{x}'\nv = f'plain'\nu = 'p' 'q'\nt = \"\"\"doc\"\"\"\n")]);
    let decls = ".decl node(d: node, n: node, k: symbol, s: number, e: number)\n.decl str(n: node, s: symbol)\n.input node, str\n.decl out(s: symbol)\n.output out\n";
    let asked = format!("{decls}out(s) :- node(_, n, \"string\", _, _), str(n, s).\n");
    let lowered = crate::code_rule_dsl::logic::rr::parse_version(&asked).unwrap();
    let inputs = crate::code_rule_dsl::logic::program_image::fields::ProgramFields::input_names(&*lowered);
    assert!(!inputs.iter().any(|i| i == "str") && lowered.rules().any(|r| lowered.rel_name(r.head().rel()) == "rr_str_req"), "str is answered on request");
    let asked = run(&dir, &asked, &["out"]);
    let eager = run(&dir, &format!("{decls}.decl all(n: node, s: symbol)\nall(n, s) :- str(n, s).\nout(s) :- all(n, s), node(_, n, \"string\", _, _).\n"), &["out"]);
    assert_eq!(asked[0].len(), 7, "string rows: {:?}", asked[0]);
    assert_eq!(asked, eager);
    let _ = std::fs::remove_dir_all(&dir);
}
