use crate::code_rule_dsl::CompiledCodeRuleDsl;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::matcher::{FileScopeSlot, UtilLimit};
use crate::code_rule_dsl::test_support::*;
use ast_grep_core::tree_sitter::LanguageExt;
use napi::Result;
use serde_json::{Value, json};

fn compile_clause(language: &str, ast: Value, violation: Value) -> Result<CompiledCodeRuleDsl> {
    let mut ast = ast;
    ast["language"] = json!(language);
    compile_document_with(
        clause_document(json!({ "id": "parse", "forbid": { "ast": ast }, "violation": violation })),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
}

fn python(rule: Value) -> CompiledCodeRuleDsl {
    compile_clause("python", json!({ "rule": rule }), violation())
        .unwrap_or_else(|error| panic!("{}", error.reason))
}

fn python_rejection(rule: Value) -> String {
    rejection(compile_clause(
        "python",
        json!({ "rule": rule }),
        violation(),
    ))
}

/// Each violation as its text and its captures' texts.
fn found(rule: Value, source: &str) -> Vec<String> {
    let result = evaluate_file(&python(rule), "python", "a.py", source);
    assert_eq!(result["failures"], json!([]), "{result}");
    matched_texts(&result, source)
}

#[test]
fn a_parsed_tree_is_matched_from_its_root() {
    let source = "a = 'xyz'\nb = 'x\\x79'\nc = 'zz'\n";
    // The rule runs on the root; relations walk the parsed tree.
    let rule = json!({ "kind": "string", "parse": {
        "grammar": "testChars", "kind": "text",
        "has": { "kind": "char", "regex": "^y$" },
    } });
    assert_eq!(found(rule, source), ["'xyz'", "'x\\x79'"]);
    // `inside` stops at the parsed root, and values are the grammar's.
    let rule = json!({ "kind": "string", "parse": {
        "grammar": "testChars",
        "has": { "kind": "char", "value": { "eq": "z" }, "inside": { "kind": "text" }, "precedes": { "value": { "eq": "z" } } },
    } });
    assert_eq!(found(rule, source), ["'zz'"]);
    let rule = json!({ "kind": "string", "parse": {
        "grammar": "testChars",
        "count": { "has": { "kind": "char" }, "min": 3 },
        "not": { "inside": { "kind": "text" } },
    } });
    assert_eq!(found(rule, source), ["'xyz'"]);
}

#[test]
fn captures_are_exported_as_mapped_spans_and_decoded_text() {
    let source = "a = 'q\\x79z'\n";
    let rule = json!({ "kind": "string", "parse": {
        "grammar": "testChars",
        "has": { "kind": "char", "value": { "eq": "y" }, "capture": "C" },
    } });
    let compiled = compile_clause(
        "python",
        json!({ "rule": rule }),
        json!({ "anchor": "capture.C", "code": "X", "message": "found $C in $S" }),
    );
    // `$S` is no capture: the parsed tree's names are the only ones exported.
    assert!(rejection(compiled).contains("`$S` names no capture; the query can bind: C"),);
    let compiled = compile_clause(
        "python",
        json!({ "rule": { "pattern": "$A = $S", "has": { "kind": "string", "parse": {
            "grammar": "testChars", "has": { "kind": "char", "value": { "eq": "y" }, "capture": "C" },
        } } } }),
        json!({ "anchor": "capture.C", "code": "X", "message": "found $C in $S", "fix": "$A = '$C'" }),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "python", "a.py", source);
    let violation = &result["violations"][0];
    // The capture anchors the escape it was decoded from; the message and fix read its value.
    let span = &violation["span"];
    let text = &source[span["startByte"].as_u64().expect("start") as usize
        ..span["endByte"].as_u64().expect("end") as usize];
    assert_eq!(text, "\\x79", "{result}");
    assert_eq!(violation["message"], "found y in 'q\\x79z'", "{result}");
    assert_eq!(violation["fix"]["text"], "a = 'y'", "{result}");
    assert_eq!(
        matched_texts(&result, source),
        ["\\x79 A=a C=\\x79 S='q\\x79z'"]
    );
}

#[test]
fn the_host_compares_exported_captures_as_text() {
    let source = "a = 'x'; b = 'x'\nc = 'x'; d = 'y'\n";
    let rule = json!({ "kind": "expression_statement",
        "has": { "kind": "assignment", "has": { "kind": "string", "parse": { "grammar": "testChars", "has": { "kind": "char", "capture": "L" } } } },
        "precedes": { "kind": "expression_statement", "has": { "kind": "assignment", "has": { "kind": "string", "parse": { "grammar": "testChars", "has": { "kind": "char", "capture": "R" } } } } },
        "compare": { "left": "L", "op": "eq", "right": "R" },
    });
    // `b = 'x'` precedes `c = 'x'`; `c` precedes `d = 'y'`, whose text differs.
    assert_eq!(found(rule, source), ["b = 'x' L=x R=x"]);
    // Inside the tree, comparisons read the tree's own captures, and values the grammar's.
    let rule = json!({ "kind": "string", "parse": { "grammar": "testChars",
        "has": { "kind": "char", "capture": "A", "precedes": { "kind": "char", "capture": "B" } },
        "compare": { "left": "A", "op": "eq", "right": "B", "text": "value" },
    } });
    assert_eq!(found(rule, "a = 'xx'\nb = 'xy'\n"), ["'xx' A=x B=x"]);
}

/// Sibling terms of a parsed tree's rule run in capture order, as a host rule's do: a `not` that
/// compares a capture a sibling relation binds runs after that relation, so it reads the capture
/// bound. Here a string matches when no char differs from the one `has` captures.
#[test]
fn a_parse_rule_negation_runs_after_the_sibling_that_binds_what_it_reads() {
    let source = "a = 'xx'\nb = 'xy'\nc = 'zzz'\n";
    let rule = json!({ "kind": "string", "parse": { "grammar": "testChars",
        "not": { "has": { "kind": "char", "capture": "C",
            "compare": { "left": "K", "op": "ne", "right": "C" } } },
        "has": { "kind": "char", "capture": "K" },
    } });
    assert_eq!(found(rule, source), ["'xx' K=x", "'zzz' K=z"]);
}

#[test]
fn text_parse_host_rules_can_call_utilities() {
    let rule = json!({
        "kind": "string",
        "parse": {
            "grammar": "testChars",
            "host": { "matches": "OTHER" }
        }
    });
    let compiled = compile_clause(
        "python",
        json!({ "rule": rule, "utils": { "OTHER": { "kind": "string" } } }),
        violation(),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "python", "a.py", "value = 'x'\n");
    assert_eq!(matched_texts(&result, "value = 'x'\n"), ["'x'"]);
}

#[test]
fn a_literal_without_a_known_value_is_unknown() {
    let source = "a = f'{x}y'\nb = 'y'\nc = x\n";
    let rule =
        json!({ "kind": "string", "parse": { "grammar": "testChars", "has": { "kind": "char" } } });
    assert_eq!(found(rule, source), ["'y'"]);
    // Unknown, not false: the negation holds on neither the f-string nor the plain string.
    let rule = json!({ "kind": "string", "not": { "parse": { "grammar": "testChars", "has": { "kind": "char", "regex": "x" } } } });
    assert_eq!(found(rule, source), ["'y'"]);
    // A node that is no string literal has no parsed tree: false.
    let rule = json!({ "kind": "identifier", "not": { "parse": { "grammar": "testChars", "kind": "text" } } });
    assert_eq!(found(rule, source), ["a", "x", "b", "c", "x"]);
}

#[test]
fn a_parse_is_sound_under_syntax_errors_elsewhere() {
    let rule =
        json!({ "kind": "string", "parse": { "grammar": "testChars", "has": { "kind": "char" } } });
    let source = "a = 'y'\ndef g(:\n";
    let result = evaluate_file(&python(rule), "python", "a.py", source);
    assert_eq!(matched_texts(&result, source), ["'y'"], "{result}");
}

#[test]
fn parses_are_charged_to_the_budget() {
    let source = "a = 'abcdefghij'\n";
    let root = GrammarDialect::Python.ast_grep(source);
    let string = root
        .root()
        .dfs()
        .find(|node| node.kind() == "string")
        .expect("string");
    let compiled =
        python(json!({ "kind": "string", "parse": { "grammar": "testChars", "kind": "text" } }));
    let grammar = &grammar_matchers(&compiled)[0];
    let _slot = FileScopeSlot::install(Vec::new(), None, false);
    let mut work = 5;
    assert_eq!(
        grammar.match_root(string.clone(), &mut work).err(),
        Some(UtilLimit::Value)
    );
    let mut work = 1_000;
    assert!(
        grammar
            .match_root(string.clone(), &mut work)
            .expect("fits")
            .is_some()
    );
    let spent = 1_000 - work;
    assert!(spent > 10, "{spent}");
    // The tree is kept for the file: a second match does not parse again.
    let mut again = 1_000;
    assert!(
        grammar
            .match_root(string, &mut again)
            .expect("fits")
            .is_some()
    );
    assert_eq!(again, 1_000);
}

#[test]
fn parse_rules_are_checked() {
    for (rule, expected) in [
        (
            json!({ "kind": "string", "capture": "S" }),
            "/clauses/0/forbid/ast/rule/capture: capture binds a node of a parsed tree; use it inside parse (a pattern's metavariables capture host nodes)",
        ),
        (
            json!({ "parse": { "grammar": "yaml", "kind": "text" } }),
            "/clauses/0/forbid/ast/rule/parse/grammar: `yaml` is not a grammar; expected one of percentFormat, identifierWords, pythonFString, braceFormat, annotation, eslintDirective, printfFormat, scanfFormat, phpDoc, rustFormat, testChars",
        ),
        (
            json!({ "parse": { "grammar": "ecmaRegex", "kind": "text" } }),
            "/clauses/0/forbid/ast/rule/parse/grammar: `ecmaRegex` is not supported yet",
        ),
        (
            json!({ "parse": { "kind": "text" } }),
            "/clauses/0/forbid/ast/rule/parse/grammar must be a non-empty string",
        ),
        (
            json!({ "parse": { "grammar": "testChars", "flags": "u", "kind": "text" } }),
            "/clauses/0/forbid/ast/rule/parse/flags: `testChars` has no flags",
        ),
        (
            json!({ "parse": { "grammar": "testChars" } }),
            "/clauses/0/forbid/ast/rule/parse must contain at least one matcher of the parsed tree",
        ),
        (
            json!({ "parse": { "grammar": "testChars", "kind": "identifier" } }),
            "/clauses/0/forbid/ast/rule/parse/kind: `identifier` is not a testChars kind; expected one of text, char, error",
        ),
        (
            json!({ "parse": { "grammar": "testChars", "pattern": "x" } }),
            "/clauses/0/forbid/ast/rule/parse/pattern: `pattern` is not supported inside parse (testChars trees)",
        ),
        (
            json!({ "parse": { "grammar": "testChars", "has": { "resolves": { "global": ["x"] } } } }),
            "/clauses/0/forbid/ast/rule/parse/has/resolves: `resolves` is not supported inside parse (testChars trees)",
        ),
        (
            json!({ "parse": { "grammar": "testChars", "kinds": "text" } }),
            "unknown field `kinds` at /clauses/0/forbid/ast/rule/parse",
        ),
        (
            json!({ "parse": { "grammar": "testChars", "has": { "kind": "char", "capture": "$C" } } }),
            "/clauses/0/forbid/ast/rule/parse/has/capture: `$C` is not a capture name (A-Z, 0-9, _, without `$`)",
        ),
        (
            json!({ "parse": { "grammar": "testChars", "value": { "eq": "x", "follow": true } } }),
            "/clauses/0/forbid/ast/rule/parse/value/follow: a parsed node has no binding to follow",
        ),
        (
            json!({ "parse": { "grammar": "testChars", "compare": { "left": "A", "op": "eq", "right": "B", "text": "tokens" } } }),
            "/clauses/0/forbid/ast/rule/parse/compare/text: a parsed tree compares by source, unquoted, value or node",
        ),
        // A comparison inside the tree reads only the tree's captures.
        (
            json!({ "pattern": "$A = $B", "has": { "kind": "string", "parse": { "grammar": "testChars", "has": { "kind": "char", "capture": "C" }, "compare": { "left": "A", "op": "eq", "right": "C" } } } }),
            "/clauses/0/forbid/ast/rule/has/parse/compare/left: `$A` is never bound where this comparison runs (names bound under `not` or `count` are visible only inside it; a stopBy rule sees only the names it binds itself)",
        ),
    ] {
        assert_eq!(python_rejection(rule.clone()), expected, "{rule}");
    }
    // Python strings only.
    assert_eq!(
        rejection(compile_clause(
            "typescript",
            json!({ "rule": { "kind": "string", "parse": { "grammar": "testChars", "kind": "text" } } }),
            violation()
        )),
        "/clauses/0/forbid/ast/rule/parse/grammar: `testChars` parses Python literals"
    );
}
