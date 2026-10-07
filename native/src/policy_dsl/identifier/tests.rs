//! NFKC identifiers against CPython 3.13 (`fixtures/python/identifier/cpython.expected.json`),
//! and one lookalike test per capability that reads Python names.

use super::*;
use crate::policy_dsl::oracle;
use crate::policy_dsl::test_support::*;
use serde_json::{Value, json};

const CPYTHON: &str = oracle!("python/identifier/cpython.expected.json");

fn cpython() -> Value {
    serde_json::from_str(CPYTHON).expect("oracle JSON")
}

/// The oracle module `name`'s source and entry.
fn module(name: &str) -> Value {
    cpython()["modules"]
        .as_array()
        .expect("modules")
        .iter()
        .find(|module| module["name"] == name)
        .unwrap_or_else(|| panic!("module {name}"))
        .clone()
}

/// What a Python rule (`ast`: `rule` and optionally `utils`, `constraints`) reports on `source`:
/// each finding's text, then its single-node captures', as `matched_texts` gives them.
fn python(ast: Value, source: &str) -> Vec<String> {
    let mut ast = ast;
    ast["language"] = json!("python");
    let document =
        check_document(json!({ "id": "nfkc", "forbid": { "ast": ast }, "violation": finding() }));
    let compiled = compile_document_with(
        document,
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "python", "nfkc_case.py", source);
    assert_eq!(result["failures"], json!([]), "{result}");
    let mut texts = matched_texts(&result, source);
    texts.sort();
    texts
}

#[test]
fn normalizes_compatibility_letters_and_borrows_normal_text() {
    // F821_28's lookalikes: each is `C` under NFKC.
    for lookalike in ["𝒞", "𝑪", "𝓒", "𝕮", "Ｃ"] {
        assert_eq!(nfkc(lookalike), "C", "{lookalike}");
    }
    assert_eq!(nfkc("ﬁle"), "file");
    assert!(matches!(nfkc("plain_ascii"), Cow::Borrowed(_)));
    // Normal non-ASCII text is borrowed too, and not folded: `é` is no `e`.
    assert!(matches!(nfkc("café"), Cow::Borrowed("café")));
    // `𝒟` is `D`, not `C`: NFKC is no confusable-folding.
    assert_eq!(nfkc("𝒟"), "D");
    assert_eq!(name_in(GrammarDialect::Python, "𝒞".into()), "C");
    assert_eq!(name_in(GrammarDialect::JavaScript, "𝒞".into()), "𝒞");
    assert_eq!(name_in(GrammarDialect::Php, "𝒞".into()), "𝒞");
}

/// Every identifier token of the oracle's modules reads as the name CPython's parser makes of it.
#[test]
fn identifiers_read_as_cpythons_parser_reads_them() {
    let oracle = cpython();
    let mut compared = 0;
    for module in oracle["modules"].as_array().expect("modules") {
        let source = module["source"].as_str().expect("source");
        for identifier in module["identifiers"].as_array().expect("identifiers") {
            let (start, end) = (
                identifier["start"].as_u64().expect("start") as usize,
                identifier["end"].as_u64().expect("end") as usize,
            );
            let text = &source[start..end];
            assert_eq!(text, identifier["text"], "{}", module["name"]);
            assert_eq!(
                nfkc(text),
                identifier["name"].as_str().expect("name"),
                "{text}"
            );
            compared += 1;
        }
    }
    assert!(compared > 50, "{compared} identifiers compared");
}

/// `resolves.unresolved` (scope table keys and lookups, imports, definitions, builtins, `global`
/// declarations) reports exactly the module-level loads CPython leaves undefined, at their text.
#[test]
fn unresolved_names_are_cpythons() {
    let oracle = cpython();
    for module in oracle["modules"].as_array().expect("modules") {
        let source = module["source"].as_str().expect("source");
        let ours = python(
            json!({ "rule": { "kind": "identifier", "resolves": { "unresolved": true } } }),
            source,
        );
        let mut theirs = module["undefined"]
            .as_array()
            .expect("undefined")
            .iter()
            .map(|name| name["text"].as_str().expect("text").to_owned())
            .collect::<Vec<_>>();
        theirs.sort();
        assert_eq!(ours, theirs, "{}", module["name"]);
    }
}

/// `__all__` entries are strings: compared as written, so `'𝒞'` names nothing the module binds
/// (CPython's star import raises `AttributeError` on it) while `'C'` names `𝒞`'s binding.
#[test]
fn dunder_all_entries_compare_raw() {
    let source = module("dunder_all")["source"]
        .as_str()
        .expect("source")
        .to_owned();
    let ours = python(
        json!({ "rule": { "kind": "string", "resolves": { "unresolved": true } } }),
        &source,
    );
    assert_eq!(ours, ["'𝒞'"]);
    let run = cpython()["runs"]
        .as_array()
        .expect("runs")
        .iter()
        .find(|run| run["name"] == "star_import")
        .expect("star_import")["outcome"]["error"]
        .clone();
    assert_eq!(
        run,
        "AttributeError: module 'nfkc_module' has no attribute '𝒞'"
    );
}

/// `resolves.import` qualifies a lookalike module, alias and attribute chain; `resolves.global`
/// reads a lookalike builtin.
#[test]
fn imports_and_globals_resolve_lookalikes() {
    let source = "import 𝐨𝐬\nfrom 𝐨𝐬 import 𝐬𝐞𝐩 as 𝒮\n𝐨𝐬.𝐩𝐚𝐭𝐡\nos.path\nS\n𝐥𝐞𝐧('ab')\n";
    let ours = python(
        json!({ "rule": { "any": [{ "kind": "identifier" }, { "kind": "attribute" }],
            "resolves": { "import": ["os.path", "os.sep"] } } }),
        source,
    );
    assert_eq!(ours, ["S", "os.path", "𝐨𝐬.𝐩𝐚𝐭𝐡"]);
    let ours = python(
        json!({ "rule": { "kind": "identifier", "resolves": { "global": ["len"] } } }),
        source,
    );
    assert_eq!(ours, ["𝐥𝐞𝐧"]);
}

/// `binds` reads usedness through lookalike reads; `resolves.binding` and `value.follow` find the
/// binding a lookalike names; a lookalike builtin constructor folds.
#[test]
fn bindings_and_values_follow_lookalikes() {
    let source = "𝒞 = 2\nprint(C)\n𝒟 = 3\n";
    let unused = python(
        json!({ "rule": { "kind": "identifier", "binds": { "used": false } } }),
        source,
    );
    assert_eq!(unused, ["𝒟"]);
    let followed = python(
        json!({ "rule": { "kind": "identifier", "regex": "^C$", "value": { "follow": true, "eq": 2 } } }),
        source,
    );
    assert_eq!(followed, ["C"]);
    let bound = python(
        json!({ "rule": { "kind": "identifier", "regex": "^C$",
            "resolves": { "binding": { "value": { "kind": "integer" } } } } }),
        source,
    );
    assert_eq!(bound, ["C"]);
    let folded = python(
        json!({ "rule": { "kind": "call", "value": { "eq": 0 } } }),
        "x = 𝐢𝐧𝐭()\n",
    );
    assert_eq!(folded, ["𝐢𝐧𝐭()"]);
}

/// `compare` `structure` and `structure-hash` agree with CPython's AST equality on each pair.
#[test]
fn structures_compare_as_cpythons_ast() {
    let oracle = cpython();
    let pairs = oracle["structures"].as_array().expect("structures");
    let source = pairs
        .iter()
        .map(|pair| {
            format!(
                "({}, {})\n",
                pair["left"].as_str().expect("left"),
                pair["right"].as_str().expect("right")
            )
        })
        .collect::<String>();
    let mut equal = pairs
        .iter()
        .filter(|pair| pair["equal"] == true)
        .map(|pair| {
            let (left, right) = (
                pair["left"].as_str().expect("left"),
                pair["right"].as_str().expect("right"),
            );
            format!("({left}, {right}) A={left} B={right}")
        })
        .collect::<Vec<_>>();
    equal.sort();
    assert!(equal.len() > 3 && equal.len() < pairs.len());
    for mode in ["structure", "structure-hash"] {
        let ours = python(
            json!({ "rule": { "pattern": "($A, $B)",
                "compare": { "left": "A", "op": "eq", "right": "B", "text": mode } } }),
            &source,
        );
        assert_eq!(ours, equal, "{mode}");
    }
}

/// `arguments` binds a lookalike keyword to its slot; a `**{...}` key is a string, compared raw,
/// as CPython's `TypeError` on `f(**{'𝐤': 1})` shows.
#[test]
fn arguments_bind_lookalike_keywords() {
    let source = "f(𝐤=1)\nf(**{'𝐤': 2})\n";
    let bound = python(
        json!({ "rule": { "kind": "call", "arguments": { "signature": ["k"], "bind": { "K": "k" } } } }),
        source,
    );
    assert_eq!(bound, ["f(𝐤=1) K=1"]);
    let absent = python(
        json!({ "rule": { "kind": "call", "arguments": { "signature": ["k"], "absent": ["k"] } } }),
        source,
    );
    assert_eq!(absent, ["f(**{'𝐤': 2})"]);
    let runs = cpython()["runs"].clone();
    let outcome = |name: &str| {
        runs.as_array()
            .expect("runs")
            .iter()
            .find(|run| run["name"] == name)
            .unwrap_or_else(|| panic!("{name}"))["outcome"]
            .clone()
    };
    assert_eq!(outcome("keyword"), json!({ "result": "1" }));
    assert_eq!(
        outcome("keyword_splat"),
        json!({ "error": "TypeError: f() got an unexpected keyword argument '𝐤'" })
    );
}

/// `formatArgs` (`str.format`): a lookalike keyword fills its field, a lookalike field name is
/// string content and stays unfilled, as CPython's `KeyError: '𝒞'` shows (Ruff's F522/F524).
#[test]
fn format_keywords_are_identifiers_and_fields_strings() {
    let source = "'{C}'.format(𝒞=1)\n'{𝒞}'.format(C=1)\n";
    let rule = |test: Value| {
        let mut test = test;
        test["style"] = json!("brace");
        test["string"] = json!("S");
        test["args"] = json!("A");
        json!({ "rule": { "pattern": "$S.format($$$A)", "formatArgs": test } })
    };
    let witness = |texts: Vec<String>| {
        texts
            .into_iter()
            .map(|text| text.split(' ').next().expect("witness").to_owned())
            .collect::<Vec<_>>()
    };
    let unused = python(
        rule(json!({ "unused": { "kind": "keyword_argument" } })),
        source,
    );
    assert_eq!(witness(unused), ["'{𝒞}'.format(C=1)"]);
    let missing = python(rule(json!({ "missing": { "kind": "field" } })), source);
    assert_eq!(witness(missing), ["'{𝒞}'.format(C=1)"]);
}

/// ECMAScript names are not normalized: ESLint's no-undef compares code units, and JavaScript
/// binds `𝒞` and `C` apart.
#[test]
fn ecmascript_names_stay_raw() {
    let document = check_document(
        json!({ "id": "raw", "forbid": { "ast": { "language": "javascript",
        "rule": { "kind": "identifier", "resolves": { "unresolved": true } } } }, "violation": finding() }),
    );
    let compiled = compile_document_with(
        document,
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let source = "const 𝒞 = 1;\n𝒞;\nC;\n";
    let result = evaluate_file(&compiled, "javascript", "a.js", source);
    assert_eq!(matched_texts(&result, source), ["C"]);
}
