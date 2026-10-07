//! A parsed tree's rule reaching the file around its literal (the fix round after the suites scored
//! M12): `host` runs a host rule on the node the `parse` holds, with the tree's captures, inside
//! the tree's own search; a directive's item `resolves` as a declared global; and `compare`'s
//! `name` reads an item's text.

use crate::policy_dsl::CompiledPolicyDsl;
use crate::policy_dsl::test_support::*;
use napi::Result;
use serde_json::{Value, json};

fn compile_check(ast: Value, finding: Value) -> Result<CompiledPolicyDsl> {
    let mut ast = ast;
    if ast.get("language").is_none() {
        ast["language"] = json!("typescript");
    }
    compile_document_with(
        check_document(json!({ "id": "host", "forbid": { "ast": ast }, "finding": finding })),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
}

fn finding(each: Option<&str>) -> Value {
    let mut finding =
        json!({ "anchor": "capture.N", "code": "X", "message": "$N", "severity": "error" });
    if let Some(each) = each {
        finding["each"] = json!(each);
    }
    finding
}

/// The messages (each item's name) of the findings, sorted.
fn names(ast: Value, each: Option<&str>, source: &str) -> Vec<String> {
    let compiled =
        compile_check(ast, finding(each)).unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "typescript", "a.ts", source);
    assert_eq!(result["failures"], json!([]), "{result}");
    let mut names = result["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|finding| finding["message"].as_str().expect("message").to_owned())
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn items(item: Value) -> Value {
    json!({ "rule": { "kind": "comment", "parse": { "grammar": "eslintDirective",
        "value": { "regex": "^globals?$" }, "has": item } } })
}

/// A `var` of the file declaring the item's name.
fn declared() -> Value {
    json!({ "inside": { "kind": "program", "has": { "stopBy": "end", "kind": "variable_declarator",
        "has": { "field": "name", "pattern": "$V" },
        "compare": { "left": "V", "op": "eq", "right": "N", "text": "name" } } } })
}

#[test]
fn a_host_rule_tests_each_candidate_item() {
    let ast = items(json!({ "kind": "item", "capture": "N", "host": declared() }));
    let source = "/*global b, a, c*/ var a, c;\n";
    // Without `each` the tree's search passes over `b`, which fails the host rule, to `a`.
    assert_eq!(names(ast.clone(), None, source), ["a"]);
    // With `each`, every item the host rule admits.
    assert_eq!(names(ast.clone(), Some("N"), source), ["a", "c"]);
    // Negated: the items no declaration names.
    let negated = items(json!({ "kind": "item", "capture": "N", "not": { "host": declared() } }));
    assert_eq!(names(negated, Some("N"), source), ["b"]);
    // A host rule reads a host capture bound before the parse, and binds nothing outward.
    let outer = json!({ "rule": { "kind": "comment", "pattern": "$C", "parse": { "grammar": "eslintDirective",
        "has": { "kind": "item", "capture": "N", "host": { "pattern": "$C" } } } } });
    assert_eq!(names(outer, None, "/* global a */\n"), ["a"]);
    let leaked = compile_check(
        items(json!({ "kind": "item", "capture": "N", "host": declared() })),
        json!({ "anchor": "capture.N", "code": "X", "message": "$V", "severity": "error" }),
    );
    assert!(rejection(leaked).starts_with("/checks/0/finding/message: `$V` names no capture"));
}

#[test]
fn an_item_resolves_as_a_declared_global() {
    let ast = |options: Value| {
        let mut ast =
            items(json!({ "kind": "item", "capture": "N", "resolves": { "global": true } }));
        ast["languageOptions"] = options;
        ast
    };
    let source = "/*global Array, a, conf, off*/\n";
    // The query's defaults: the latest ecmaVersion's builtins.
    let defaults = items(json!({ "kind": "item", "capture": "N", "resolves": { "global": true } }));
    assert_eq!(names(defaults, Some("N"), source), ["Array"]);
    // Configured globals count, `off` ones do not, and an ecmaVersion's builtins are its own.
    let configured = ast(json!({ "ecmaVersion": 5, "sourceType": "script",
        "globals": { "conf": "readonly", "off": "off" } }));
    assert_eq!(names(configured, Some("N"), source), ["Array", "conf"]);
    // A list asks about its names: without languageOptions any listed name, with them a listed
    // declared global.
    let listed = |options: Option<Value>| {
        let mut ast = items(
            json!({ "kind": "item", "capture": "N", "resolves": { "global": ["Array", "a"] } }),
        );
        if let Some(options) = options {
            ast["languageOptions"] = options;
        }
        ast
    };
    assert_eq!(names(listed(None), Some("N"), source), ["Array", "a"]);
    assert_eq!(names(listed(Some(json!({}))), Some("N"), source), ["Array"]);
}

#[test]
fn compare_name_reads_an_items_text() {
    // `a` is named `a`; the item is its text.
    let ast = |text: &str| {
        json!({ "rule": { "kind": "comment", "parse": { "grammar": "eslintDirective",
            "has": { "kind": "item", "capture": "N" } },
            "inside": { "kind": "program", "has": { "stopBy": "end", "kind": "identifier", "pattern": "$V",
                "compare": { "left": "V", "op": "eq", "right": "N", "text": text } } } } })
    };
    let source = "/* global a */\n\\u0061;\n";
    assert_eq!(names(ast("name"), None, source), ["a"]);
    assert!(names(ast("source"), None, source).is_empty());
}

#[test]
fn host_and_item_resolves_are_refused_where_they_mean_nothing() {
    let refuse = |ast: Value| rejection(compile_check(ast, finding(None)));
    assert_eq!(
        refuse(json!({ "rule": { "kind": "comment", "host": { "kind": "program" } } })),
        "/checks/0/forbid/ast/rule/host: host tests the literal a parse holds; use it inside parse"
    );
    assert_eq!(
        refuse(
            json!({ "language": "python", "rule": { "kind": "string", "parse": { "grammar": "annotation",
            "kind": "identifier", "host": { "kind": "module" } } } })
        ),
        "/checks/0/forbid/ast/rule/parse/host: `host` is not supported inside parse (annotation trees)"
    );
    assert_eq!(
        refuse(
            json!({ "language": "python", "rule": { "kind": "call", "formatArgs": { "style": "brace",
            "string": "S", "args": "A", "missing": { "kind": "field", "host": { "kind": "module" } } } } })
        ),
        "/checks/0/forbid/ast/rule/formatArgs/missing/host: host tests the literal a parse holds; formatArgs' fields have none"
    );
    assert_eq!(
        refuse(items(
            json!({ "kind": "item", "resolves": { "unresolved": true } })
        )),
        "/checks/0/forbid/ast/rule/parse/has/resolves/unresolved: a directive's node resolves by `global` only: its text as a declared global of the query's languageOptions"
    );
    assert_eq!(
        refuse(
            json!({ "language": "python", "rule": { "kind": "string", "parse": { "grammar": "percentFormat",
            "kind": "placeholder", "resolves": { "global": true } } } })
        ),
        "/checks/0/forbid/ast/rule/parse/resolves: `resolves` is not supported inside parse (percentFormat trees)"
    );
    // A comparison inside a host rule reads the tree's captures and the host's, and no other.
    assert_eq!(
        refuse(items(
            json!({ "kind": "item", "capture": "N", "host": { "kind": "comment",
            "compare": { "left": "Q", "op": "eq", "right": "N" } } })
        )),
        "/checks/0/forbid/ast/rule/parse/has/host/compare/left: `$Q` is never bound where this comparison runs (names bound under `not` or `count` are visible only inside it; a stopBy rule sees only the names it binds itself)"
    );
}

#[test]
fn a_host_rule_is_as_sound_as_it_reads() {
    let sound = |item: Value| {
        let plain =
            json!({ "anchor": "witness.span", "code": "X", "message": "m", "severity": "error" });
        let compiled =
            compile_check(items(item), plain).unwrap_or_else(|error| panic!("{}", error.reason));
        grammar_matchers(&compiled)[0].sound_under_errors
    };
    // Outward and positive: a fact of the recovered tree.
    assert!(sound(
        json!({ "kind": "item", "capture": "N", "host": declared() })
    ));
    // Negated, it claims an absence around the literal.
    assert!(!sound(
        json!({ "kind": "item", "capture": "N", "not": { "host": declared() } })
    ));
    // A host rule unsound alone is unsound here.
    assert!(!sound(
        json!({ "kind": "item", "host": { "not": { "inside": { "kind": "program" } } } })
    ));
    // An item's global reads the configuration only.
    assert!(sound(
        json!({ "kind": "item", "not": { "resolves": { "global": true } } })
    ));
}
