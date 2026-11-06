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
    let capabilities: Value = serde_json::from_str(CAPABILITIES).unwrap();
    let compiled = compile_document_with(clause_document(json!({ "id": "c", "forbid": forbid, "violation": violation() })), &capabilities)
        .unwrap_or_else(|e| panic!("{}", e.reason));
    evaluate_file(&compiled, "python", path, source)["violations"].as_array().unwrap().clone()
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

/// W's findings of a Python `ast` rule over `files`, each `path\tline\tcol` (1-based line, byte column).
fn w_at(rule: Value, files: &[(&str, &str)]) -> Vec<String> {
    let mut out = Vec::new();
    for (path, source) in files {
        for v in w(json!({ "ast": { "language": "python", "rule": rule.clone() } }), path, source) {
            out.push(format!("{path}\t{}\t{}", v["span"]["startLine"].as_u64().unwrap() + 1, v["span"]["startColumn"].as_u64().unwrap()));
        }
    }
    out
}

/// A module with nested definitions, statements side by side, and a class.
const TREE: (&str, &str) = (
    "t.py",
    "def f(a):\n    x = g(a)\n    y = h(x)\n    print(y)\n    return y\n\n\nclass C:\n    def m(self):\n        z = k(1)\n        z.w()\n        return z\n",
);

/// core.rr's tree arithmetic, which lowering rewrites to the base relations (`rr::lower::core`), as W's matcher
/// sees it: `kind` is a node's kind; `within` W's `inside` to the end, its ancestor's kind tested (a kind set of
/// one) or read from a relation (the set of every kind); `next_sibling` W's `follows` at a neighbour (statements of a
/// block, which no token separates), negated W's `nthChild: 1`; `precedes` W's `precedes` to the end. No program
/// declares them: core.rr's declarations are the prelude.
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
    let parsed = crate::code_rule_dsl::logic::rr::parse(program).unwrap();
    assert!(parsed.kindsets.contains_key("rr_kind_function_definition") && parsed.kindsets.contains_key("rr_kind_any"), "{:?}", parsed.kindsets);
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
        assert_eq!(*got, csv(want));
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
    assert_eq!(got[0], "-1\n");
    assert_eq!(got[1..4], ["", "", ""]);
    assert_eq!(got[4], csv(w_at(json!({ "kind": "call" }), &files)));
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
    let want = [
        "binary_operator\t5\t0\t*",
        "binary_operator\t5\t0\t+",
        "boolean_operator\t6\t0\tor",
        "comparison_operator\t1\t0\t<",
        "comparison_operator\t1\t1\t<=",
        "comparison_operator\t3\t0\tnot in",
        "comparison_operator\t4\t0\tis not",
    ];
    assert_eq!(got[0], csv(want.map(str::to_owned)));
    for op in ["<", "<=", "not in", "is not"] {
        let escaped = regex::escape(op);
        let at = w_at(json!({ "kind": "comparison_operator", "has": { "field": "operators", "regex": format!("^{escaped}$") } }), &files);
        let first: Vec<String> = want.iter().filter_map(|r| r.strip_prefix("comparison_operator\t")?.strip_suffix(&format!("\t0\t{op}"))).map(|l| format!("o.py\t{l}\t{}", if l == "1" { 3 } else { 4 })).collect();
        assert_eq!(at, first, "{op}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// `prev_token(n, s)` is W's `follows` at a neighbour, anonymous tokens included (`,` and `import` in
/// sql-grammar-escape-hatches' usage clauses); `token(p, i, s, start, end)` read at the next position is W's
/// `precedes` at a neighbour.
#[test]
fn tokens_are_ws_neighbours() {
    let files = [("i.py", "from m import a, b\nimport x, y.z\nf(a, b, c=1)  # done\n")];
    let dir = corpus("tokens", &files);
    let kinds = ["identifier", "dotted_name", "keyword_argument"];
    let tokens = [",", "import", "(", ")", "from"];
    let facts = |rel: &str, items: &[&str]| items.iter().map(|k| format!("{rel}({k:?}).")).collect::<Vec<_>>().join("\n");
    let program = format!(
        r#"
.decl kinds(k: symbol)
{}
.decl tokens(t: symbol)
{}
.decl after(k: symbol, t: symbol, l: number, c: number)
after(k, t, l, c) :- kinds(k), kind(n, k), prev_token(n, t), tokens(t), line(n, l), col(n, c).
.decl before(k: symbol, t: symbol, l: number, c: number)
before(k, t, l, c) :- kinds(k), node(_, n, k, s, e), child(p, n, _), token(p, i, _, s, e), j = i + 1, token(p, j, t, _, _), tokens(t),
    line(n, l), col(n, c).
.output after, before
"#,
        facts("kinds", &kinds),
        facts("tokens", &tokens)
    );
    let got = run(&dir, &program, &["after", "before"]);
    let (mut after, mut before) = (Vec::new(), Vec::new());
    for k in kinds {
        for t in tokens {
            let regex = format!("^{}$", regex::escape(t));
            let at = |rule: Value| w_at(rule, &files).into_iter().map(|r| format!("{k}\t{t}\t{}", r.split_once('\t').unwrap().1)).collect::<Vec<_>>();
            after.extend(at(json!({ "kind": k, "follows": { "stopBy": "neighbor", "regex": regex } })));
            before.extend(at(json!({ "kind": k, "precedes": { "stopBy": "neighbor", "regex": regex } })));
        }
    }
    assert!(after.iter().any(|r| r.starts_with("dotted_name\t,\t")) && after.iter().any(|r| r.starts_with("dotted_name\timport\t")), "{after:?}");
    assert_eq!(got, [csv(after), csv(before)]);
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
                parts.push(format!("{path}\t{c}"));
            }
            if in_scope(&format!("**/{c}"), path, source) {
                names.push(format!("{path}\t{c}"));
            }
        }
    }
    assert_eq!(got, [csv(parts), csv(names)]);
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
    let got = run(&dir, program, &["visits", "later"]);
    let mut first: std::collections::BTreeMap<(i64, i64), i64> = std::collections::BTreeMap::new();
    let mut indexes = BTreeSet::new();
    for row in got[0].lines() {
        let f: Vec<i64> = row.split('\t').map(|x| x.parse().unwrap()).collect();
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
    assert!(got[1].lines().any(|l| l == "173\t1"), "{}", got[1]);
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
    assert_eq!(got[0], csv(want));
    let others = json!({ "kind": "identifier", "inside": { "not": { "any": [{ "kind": "call" }, { "kind": "attribute" }] } } });
    assert_eq!(got[1], csv(w_at(others, &files)));
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
    assert_eq!(got[0], "");
    assert_eq!(got[1].lines().count(), 5, "{}", got[1]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A relation nothing derives and the runner does not bind is a named error, not the planner's missing statistics;
/// those that need CPython say so.
#[test]
fn an_unbound_relation_is_named() {
    let dir = corpus("unbound", &[TREE]);
    let out = dir.join(".out");
    let fails = |program: &str| super::run(program, &dir, None, &out, "program", Binding::Cpu, 1 << 18).err().map(|e| e.to_string()).unwrap_or_default();
    let e = fails(".decl v(n: node)\nv(n) :- value(n, _).\n.output v\n");
    assert!(e.contains("value is read, and nothing derives it or binds it"), "{e}");
    let e = fails(".decl v(n: node)\nv(n) :- str_part(n, 0, _, _, _).\n.output v\n");
    assert!(e.contains("str_part is read") && e.contains("CPython"), "{e}");
    let e = fails(".decl v(s: symbol)\nv(@unparse(n)) :- kind(n, \"call\").\n.output v\n");
    assert!(e.contains("@unparse is called, and it needs CPython"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `as` names a variable (python/calls.rr's `field(c, "arguments", as)`), as the engine always read it. Soufflé
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
    assert_eq!(run(&dir, program, &["args"])[0], csv(w_at(json!({ "kind": "argument_list", "inside": { "kind": "call" } }), &files)));
    let _ = std::fs::remove_dir_all(&dir);
}
