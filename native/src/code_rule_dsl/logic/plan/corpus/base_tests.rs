//! core.rr's base relations the corpus runner binds beyond T's (`facts::vtab::syntax::Extra`), each against W's
//! notion of it (`CodeRuleDslRuntime`, the matcher, the regex query) on fixture files.

use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::model::{RegexLines, RegexOptions};
use crate::code_rule_dsl::test_support::{CAPABILITIES, clause_document, compile_document_with, evaluate_file, violation};
use crate::code_rule_dsl::typing::file_local::Binding;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// `files` (path, source) written under a fresh directory named for `name`.
fn corpus(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension(format!("base-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    for (path, source) in files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, source).unwrap();
    }
    dir
}

/// `program`'s outputs over the corpus in `dir`, by relation: each `<rel>.csv` as the runner writes it.
fn run(dir: &std::path::Path, program: &str, outputs: &[&str]) -> Vec<String> {
    let out = dir.join(".out");
    super::run(program, dir, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    outputs.iter().map(|rel| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap_or_default()).collect()
}

/// Rows as the runner writes a relation: sorted, deduplicated, each ended by `\n`.
fn csv(rows: impl IntoIterator<Item = String>) -> String {
    let rows: BTreeSet<String> = rows.into_iter().collect();
    rows.into_iter().map(|r| r + "\n").collect()
}

/// W's findings of one Python `clause` (its `forbid`, at `violation()`) in `source`.
fn w(forbid: Value, path: &str, source: &str) -> Vec<Value> {
    w_as(forbid, "python", path, source)
}

/// W's findings of one clause in `source`, a file of language `language` (one with no grammar: "text").
fn w_as(forbid: Value, language: &str, path: &str, source: &str) -> Vec<Value> {
    let capabilities: Value = serde_json::from_str(CAPABILITIES).unwrap();
    let compiled = compile_document_with(clause_document(json!({ "id": "c", "forbid": forbid, "violation": violation() })), &capabilities)
        .unwrap_or_else(|e| panic!("{}", e.reason));
    evaluate_file(&compiled, language, path, source)["violations"].as_array().unwrap().clone()
}

/// W's spans of a content regex's findings in `source` (`lines`: a `lines` regex's), each as `path`, start line and
/// end line (1-based), start column and, with `text`, the text matched.
fn w_regex(pattern: &str, lines: bool, language: &str, path: &str, source: &str, text: bool) -> Vec<String> {
    let mut regex = json!({ "source": "file.content", "pattern": pattern });
    if lines {
        regex["lines"] = json!({});
    }
    w_as(json!({ "regex": regex }), language, path, source)
        .into_iter()
        .map(|v| {
            let at = |k: &str| v["span"][k].as_u64().unwrap() as usize;
            let row = format!("{path}\t{}\t{}\t{}", at("startLine") + 1, at("endLine") + 1, at("startColumn"));
            if !text {
                return row;
            }
            // The span's bytes are the file's, in a line too (its line and column are the line's).
            format!("{row}\t{}", &source[at("startByte")..at("endByte")])
        })
        .collect()
}

/// Every terminator `str.splitlines()` knows, a `\r\n`, a blank line, and no final terminator.
const LINES: &str = "a = 1\r\nb = 2\rc = 3\x0bd = 4\x0ce = 5\x1cf\x1dg\x1eh\u{85}i\u{2028}j\u{2029}k = 6\n\n  last = 0";

/// W's lines of a `lines` regex (`query::regex_query_matches`), 1-based, with their text.
fn w_lines(source: &str, lf_only: bool) -> Vec<String> {
    let options = RegexOptions {
        lines: Some(RegexLines { include: vec![], exclude: vec![], first: true, lf_only, state: None, indentation: None }),
        ..Default::default()
    };
    let (matches, complete) = crate::code_rule_dsl::query::regex_query_matches(&regex::Regex::new("(?s).*").unwrap(), source, &options, usize::MAX);
    assert!(complete);
    matches.into_iter().map(|m| format!("{}\t{}", m.line.unwrap() + 1, &source[m.context])).collect()
}

/// `src_line` is W's lines of a `lines` regex (Python's `str.splitlines()`), `src_line_lf` its `terminators: "lf"`
/// lines, `line_count` their number; a line span's `text` is the line without its terminator, its `line` and
/// `end_line` its number, its `col` 0.
#[test]
fn source_lines_are_ws_regex_lines() {
    let dir = corpus("lines", &[("a.py", LINES)]);
    let program = r#"
.decl src_line(d: number, l: number, sp: span)
.decl src_line_lf(d: number, l: number, sp: span)
.decl line_count(d: number, k: number)
.decl text(n: loc, s: symbol)
.decl line(n: loc, l: number)
.decl end_line(n: loc, l: number)
.decl col(n: loc, c: number)
.input src_line, src_line_lf, line_count, text, line, end_line, col
.decl lines(l: number, s: symbol)
lines(l, s) :- src_line(_, l, sp), text(sp, s), line(sp, l), end_line(sp, l), col(sp, 0).
.decl lf_lines(l: number, s: symbol)
lf_lines(l, s) :- src_line_lf(_, l, sp), text(sp, s), line(sp, l), end_line(sp, l), col(sp, 0).
.decl count(k: number)
count(k) :- line_count(_, k).
.output lines, lf_lines, count
"#;
    let got = run(&dir, program, &["lines", "lf_lines", "count"]);
    let (python, lf) = (w_lines(LINES, false), w_lines(LINES, true));
    assert_eq!(python.len(), 13, "{python:?}");
    assert_eq!(got[2], format!("{}\n", python.len()));
    assert_eq!(got[0], csv(python));
    assert_eq!(got[1], csv(lf));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A `lines` regex clause as `src_line`, `text` and `match`: W's findings, line for line.
#[test]
fn a_lines_regex_is_src_line_text_and_match() {
    let files = [("a.py", "x = needle\n# needle\n  y = needle; needle\r\nz = 1\x0cneedle = 2\n"), ("b/c.py", "no = 1\n")];
    let dir = corpus("lines-regex", &files);
    let program = r#"
.decl doc(d: number, path: symbol, module: symbol)
.decl src_line(d: number, l: number, sp: span)
.decl text(n: loc, s: symbol)
.input doc, src_line, text
.decl hit(p: symbol, l: number)
hit(p, l) :- doc(d, p, _), src_line(d, l, sp), text(sp, s), match(".*needle.*", s), !match("\\s*#.*", s).
.output hit
"#;
    let got = run(&dir, program, &["hit"]);
    let forbid = json!({ "regex": { "source": "file.content", "pattern": "needle", "lines": { "exclude": ["^\\s*#"] } } });
    let want = files.iter().flat_map(|(path, source)| {
        w(forbid.clone(), path, source).into_iter().map(move |v| format!("{path}\t{}", v["span"]["startLine"].as_u64().unwrap() + 1))
    });
    let want = csv(want);
    assert_eq!(want, "a.py\t1\na.py\t3\na.py\t5\n");
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
    let want = csv(files.iter().filter(|(path, source)| !w(forbid.clone(), path, source).is_empty()).map(|(path, _)| (*path).to_owned()));
    assert_eq!(want, "a.py\n");
    assert_eq!(got[0], want);
    // W's span of the whole file (`\A(?s:.*)\z`): its end line is the line holding the end.
    let whole = json!({ "regex": { "source": "file.content", "pattern": "\\A(?s:.*)\\z" } });
    let want = csv(files.iter().map(|(path, source)| {
        let [v] = w(whole.clone(), path, source).try_into().unwrap();
        format!("{path}\t{}\t{}", v["span"]["startLine"].as_u64().unwrap() + 1, v["span"]["endLine"].as_u64().unwrap() + 1)
    }));
    assert_eq!(got[1], want);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `tape_line(d, l, sp)`: the lines `line(n, l)` numbers (the tape's line table, W's `layout` lines), the empty
/// line after a final terminator included.
#[test]
fn tape_lines_are_the_line_table() {
    let source = "a = 1\r\nb = 2\rc = 3\x0cd = 4\n";
    let dir = corpus("tape-lines", &[("a.py", source)]);
    let program = r#"
.decl tape_line(d: number, l: number, sp: span)
.decl text(n: loc, s: symbol)
.input tape_line, text
.decl lines(l: number, s: symbol)
lines(l, s) :- tape_line(_, l, sp), text(sp, s).
.output lines
"#;
    let got = run(&dir, program, &["lines"]);
    let table = LineTable::new(source, Terminators::of(GrammarDialect::Python));
    let want = csv((0..table.starts().len()).map(|i| format!("{}\t{}", i + 1, &source[table.start(i)..table.end(i)])));
    assert_eq!(got[0], want);
    assert!(want.starts_with("1\ta = 1\n") && want.contains("4\t\n"), "{want:?}");
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
                want.push(format!(
                    "{path}\t{kind}\t{}\t{}\t{}\t{}",
                    at("startLine") + 1,
                    at("endLine") + 1,
                    at("startColumn"),
                    &source[at("startByte") as usize..at("endByte") as usize]
                ));
            }
        }
    }
    assert!(want.iter().any(|r| r.starts_with("a.py\tmodule\t3\t")), "the module starts at its first token: {want:?}");
    assert_eq!(got[0], csv(want));
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
    assert_eq!(run(&dir, program, &["t"])[0], "x\t1\ny\t2\n");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file whose tree starts with an ERROR node (its kind atom past the dialect's stride) is read in its own dialect:
/// `node` has its ERROR nodes, by kind as W's `kind: "ERROR"` finds them, and `syntax_error` has the file.
#[test]
fn a_tree_rooted_at_an_error_node_is_read() {
    let (path, source) = ("l9750_graphql/d.py", "def f(scope: PlanScope):\n    return (\n");
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
    let want = csv(w(json!({ "ast": { "language": "python", "syntax": "invalid", "rule": { "kind": "ERROR" } } }), path, source).into_iter().map(|v| {
        let span = &v["span"];
        format!("{path}\t{}\t{}\t{}", span["startLine"].as_u64().unwrap() + 1, span["startByte"], span["endByte"])
    }));
    assert!(want.starts_with(&format!("{path}\t1\t0\t")), "W finds the ERROR root: {want:?}");
    assert_eq!(got[0], want);
    assert_eq!(got[1], format!("{path}\n"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A base relation a program reads is an input though its `.input` leaves it out, as in an authored file: `text`
/// over spans only (the program port-21 reported failing with "no such column: n in SELECT n, s FROM text"), in
/// rules run per chunk and in rules run once over every file.
#[test]
fn base_relations_read_are_inputs() {
    let files = [("l9750_graphql/bad.py", "def f(:\n    ServerPlan\n"), ("l9750_graphql/ok.py", "x = 1\n")];
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
    assert_eq!(got[0], "l9750_graphql/bad.py\n");
    assert_eq!(got[1], "l9750_graphql/ok.py\n");
    assert_eq!(got[2], csv(files.iter().map(|(p, _)| format!("{p}\t{p}"))));
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
        assert_eq!(run(dir, program, &["calls", "lines"]), ["", ""], "{}", dir.display());
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
    let all = |pattern: &str, lines: bool, text: bool| csv(files.iter().flat_map(|(path, source)| w_regex(pattern, lines, "python", path, source, text)));
    let whole = all("needle\\w*", false, true);
    assert_eq!(whole.lines().count(), 5, "{whole}");
    assert_eq!(got[0], whole);
    let across = all("(?s)needle.end", false, false);
    assert_eq!(across, "a.py\t4\t5\t7\n");
    assert_eq!(got[1], across);
    assert_eq!(got[2], all("needle\\w*", true, true));
    let pairs = all("aa", false, true);
    assert_eq!(pairs, "b.py\t1\t1\t0\taa\nb.py\t1\t1\t10\taa\nb.py\t1\t1\t2\taa\nb.py\t1\t1\t8\taa\n");
    assert_eq!(got[3], pairs);
    assert_eq!(got[4], "b.py\n");
    let _ = std::fs::remove_dir_all(&dir);
}
