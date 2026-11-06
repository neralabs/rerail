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

/// `program`'s `vals(s, e, v)` rows read back as `(start, end, v)`.
fn spans_and_values(csv: &str) -> Vec<(u64, u64, String)> {
    csv.lines()
        .map(|row| {
            let mut cells = row.splitn(3, '\t');
            let mut number = || cells.next().unwrap().parse::<u64>().unwrap();
            let (start, end) = (number(), number());
            (start, end, cells.next().unwrap().to_owned())
        })
        .collect()
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
    let got = spans_and_values(&run(&dir, program, &["vals"])[0]);
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
    let out = run(&dir, program, &["vals", "plain"]);
    let (got, plain) = (spans_and_values(&out[0]), spans_and_values(&out[1]));
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
    let out = run(&dir, program, &["types", "pairs", "hits", "vals"]);
    let spans = |f: &Value| (f["span"]["startByte"].as_u64().unwrap(), f["span"]["endByte"].as_u64().unwrap());
    let mut want = Vec::new();
    for ty in TYPES {
        for f in w(json!({ "ast": { "language": "python", "rule": { "value": { "type": ty } } } }), "a.py", VALUES) {
            let (s, e) = spans(&f);
            want.push(format!("{s}\t{e}\t{ty}"));
        }
    }
    assert!(want.iter().any(|r| r.ends_with("\tlist")) && want.iter().any(|r| r.ends_with("\tbytes")), "{want:?}");
    assert_eq!(out[0], csv(want.clone()));
    // value_type: each typed node's value text with its type.
    let values = spans_and_values(&out[3]);
    let pairs = want.iter().flat_map(|row| {
        let [s, e, ty]: [&str; 3] = row.split('\t').collect::<Vec<_>>().try_into().unwrap();
        let (s, e) = (s.parse::<u64>().unwrap(), e.parse::<u64>().unwrap());
        values.iter().filter(move |(vs, ve, _)| (*vs, *ve) == (s, e)).map(move |(_, _, v)| format!("{v}\t{ty}"))
    });
    let pairs = csv(pairs);
    assert!(pairs.contains("12\tstr\n") && pairs.contains("12\tint\n"), "{pairs}");
    assert_eq!(out[1], pairs);
    let regex = w(json!({ "ast": { "language": "python", "rule": { "value": { "regex": "b" } } } }), "a.py", VALUES);
    let regex = csv(regex.iter().map(|f| {
        let (s, e) = spans(&f);
        format!("{s}\t{e}")
    }));
    assert!(!regex.is_empty());
    assert_eq!(out[2], regex);
    let _ = std::fs::remove_dir_all(&dir);
}
