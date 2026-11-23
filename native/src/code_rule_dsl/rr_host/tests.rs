//! What `describeRr` and `runRr` report, read through their entries: a rule file's facts, a run that goes on past a
//! policy that fails, and each finding's span.

use super::{describe, run, spans::FileSpans};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// A rule's own library file, kept beside it under `lib/`, as a pack keeps one.
const CALLS: &str = ".decl shout_call(c: node)\nshout_call(c) :- kind(c, \"call\"), field(c, \"function\", f), text(f, \"shout\").\n";

/// Each call of `shout()`, through the library file beside the rule.
const SHOUT: &str = ".policy \"rr-host-shout\" revision 2
.description \"Calls of shout().\"
.language python
.mode log
.scope include \"**/*.py\"
.scope exclude \"vendor/**\"

#include \"lib/calls.rerail\"

.violation Shout(at: node) \"shout() is called here\"
    suppress ruff \"T201\"
Shout(c) :- shout_call(c).

.test one
    .fixture \"a.py\" `shout(1)`
    .expect Shout at \"a.py\":1
.end
";

/// A valid program that declares no finding: its run fails (`tools::run_rr`).
const QUIET: &str = ".policy \"rr-host-quiet\" revision 1\n.language python\n.mode log\n.decl q(n: node)\nq(n) :- kind(n, \"call\").\n";

/// A rule narrowed by every `.scope` form that selects documents.
const NARROWED: &str = ".policy \"rr-host-narrowed\" revision 1
.language python
.mode log
.scope include \"**/*.py\"
.scope dirs \"shop\"
.scope exclude \"vendor/**\"
.scope tests false
.scope text \"**/*.md\"

.violation Narrowed(at: node) \"narrowed\"
Narrowed(c) :- kind(c, \"call\").
";

const MULTI_BYTE: &str = "greeting = \"héllo wörld 漢字\"; shout(\"ü\")\n";
const MULTI_LINE: &str = "def f():\n    return shout(\n        1,\n    )\n";

/// A fresh folder for one test, with `files` written in it.
fn folder(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let root = crate::code_rule_dsl::fixtures::temp().with_extension(name);
    let _ = std::fs::remove_dir_all(&root);
    for (path, text) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().expect("a file's folder")).unwrap();
        std::fs::write(path, text).unwrap();
    }
    dunce::canonicalize(&root).unwrap()
}

/// `run::run` of `policies` over `root`, its outputs under `root/../<root>.out`.
fn run_over(root: &Path, policies: &[PathBuf]) -> Value {
    let out = root.with_extension("out");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let request = json!({"root": root, "policies": policies, "out": out});
    let result = run::run(&request.to_string()).unwrap();
    let _ = std::fs::remove_dir_all(&out);
    serde_json::from_str(&result).unwrap()
}

fn byte_offset(text: &str, needle: &str) -> usize {
    text.find(needle).expect("the needle")
}

#[test]
fn describe_reads_what_a_rule_states_and_names_why_a_file_is_refused() {
    let root = folder(
        "rr-host-describe",
        &[
            ("lib/calls.rerail", CALLS),
            ("shout.rerail", SHOUT),
            ("bad.rerail", ".policy \"bad\" revision 1\nBad(c) :- kind(c, \"call\"\n"),
            ("coded.rerail", ".policy \"coded\" revision 1\n.language python\n.mode steer\n.violation Coded(at: node, c: symbol) code {c} \"x\"\nCoded(n, \"A1\") :- kind(n, \"call\").\n"),
            ("narrowed.rerail", NARROWED),
        ],
    );
    let rows = describe::describe(&[root.join("shout.rerail"), root.join("bad.rerail"), root.join("coded.rerail"), root.join("narrowed.rerail")]);
    let _ = std::fs::remove_dir_all(&root);
    let row = &rows[0];
    assert_eq!(row["ok"], true, "{row:#}");
    assert_eq!([&row["id"], &row["revision"], &row["description"], &row["mode"], &row["language"]], [&json!("rr-host-shout"), &json!(2), &json!("Calls of shout()."), &json!("log"), &json!("python")]);
    assert_eq!(row["scope"], json!({"include": ["**/*.py"], "exclude": ["vendor/**"], "text": [], "toolCalls": false}));
    assert_eq!([&row["codes"], &row["tests"], &row["suppresses"], &row["fixes"]], [&json!(["Shout"]), &json!(1), &json!(true), &json!(false)]);
    let files: Vec<&str> = row["files"].as_array().unwrap().iter().map(|file| file["path"].as_str().unwrap()).collect();
    assert!(files[0].ends_with("shout.rerail") && files.iter().any(|file| file.ends_with("lib/calls.rerail")), "{files:?}");
    let bad = rows[1]["error"].as_str().unwrap();
    assert_eq!((&rows[1]["ok"], bad.matches("bad.rerail").count()), (&json!(false), 1), "{bad}");
    let coded = rows[2]["error"].as_str().unwrap();
    assert!(rows[2]["ok"] == false && coded.contains(".violation Coded: its code is a column"), "{coded}");
    // Every form that narrows the documents read, as the run applies them: `dirs` as its folder's glob, `tests false`
    // as the test folders', and `text`'s globs beside them.
    assert_eq!(rows[3]["ok"], true, "{:#}", rows[3]);
    assert_eq!(
        rows[3]["scope"],
        json!({
            "include": ["**/*.py", "shop/**"],
            "exclude": ["**/test/**", "**/tests/**", "test/**", "tests/**", "vendor/**"],
            "text": ["**/*.md"],
            "toolCalls": false,
        })
    );
}

#[test]
fn a_run_goes_on_past_a_policy_that_fails_and_spans_each_finding() {
    let root = folder("rr-host-run", &[("src/text.py", MULTI_BYTE), ("src/call.py", MULTI_LINE)]);
    let rules = folder("rr-host-run-rules", &[("lib/calls.rerail", CALLS), ("shout.rerail", SHOUT), ("quiet.rerail", QUIET)]);
    let result = run_over(&root, &[rules.join("quiet.rerail"), rules.join("shout.rerail")]);
    let _ = (std::fs::remove_dir_all(&root), std::fs::remove_dir_all(&rules));
    let failures = result["failures"].as_array().unwrap();
    assert_eq!(failures.len(), 1, "{result:#}");
    assert!(failures[0]["policy"].as_str().unwrap().ends_with("quiet.rerail") && failures[0]["message"].as_str().unwrap().contains("no .violation"), "{failures:?}");
    let finding = |path: &str| result["findings"].as_array().unwrap().iter().find(|finding| finding["path"] == path).cloned().unwrap_or_else(|| panic!("no finding in {path}: {result:#}"));
    let at = byte_offset(MULTI_BYTE, "shout(");
    let end = at + "shout(\"ü\")".len();
    assert_eq!(finding("src/text.py")["span"], json!({"startByte": at, "endByte": end, "startLine": 0, "startColumn": at, "endLine": 0, "endColumn": end}));
    let at = byte_offset(MULTI_LINE, "shout(");
    assert_eq!(finding("src/call.py")["line"], 2);
    assert_eq!(finding("src/call.py")["span"], json!({"startByte": at, "endByte": MULTI_LINE.rfind(')').unwrap() + 1, "startLine": 1, "startColumn": "    return ".len(), "endLine": 3, "endColumn": 5}));
}

#[test]
fn a_finding_at_a_line_spans_its_text_and_one_at_the_file_its_start() {
    let root = folder("rr-host-lines", &[("m.py", "x = 1\ny = \"é\"\n")]);
    let file = FileSpans::read(&root, "m.py", Some("python")).unwrap();
    let _ = std::fs::remove_dir_all(&root);
    let line = file.span(None, 2).unwrap();
    assert_eq!((line.start_byte, line.end_byte, line.start_line, line.start_column, line.end_line, line.end_column), (6, 14, 1, 0, 1, 8));
    let whole = file.span(None, 0).unwrap();
    assert_eq!((whole.start_byte, whole.end_byte, whole.start_line, whole.end_line), (0, 0, 0, 0));
    assert!(file.span(None, 3).is_none());
}
