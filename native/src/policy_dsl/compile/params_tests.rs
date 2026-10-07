//! Parameterized utilities: `{ "params": [...], "rule": ... }` called as
//! `{ "matches": { "id": { "PARAM": "$CAPTURE" } } }`, each call a frame of its own.

use crate::policy_dsl::test_support::*;
use serde_json::{Value, json};

/// Every `a = b` pair's right side where the left side at the same position has the same name, as
/// ESLint's no-self-assign pairs array destructuring, to any depth: `pair` walks up from a right
/// node and returns its mirror on the left, a frame per level.
fn self_assign() -> Value {
    json!({
        "rule": {
            "kind": "identifier",
            "pattern": "$R",
            "matches": { "pair": { "X": "$R", "Y": "$L" } },
            "compare": { "left": "L", "op": "eq", "right": "R", "text": "name" },
        },
        "utils": {
            "pair": { "params": ["X", "Y"], "rule": {
                "pattern": "$X",
                "any": [
                    { "inside": { "field": "right", "kind": "assignment_expression", "has": { "field": "left", "pattern": "$Y" } } },
                    { "all": [
                        { "inside": { "kind": "array", "matches": { "pair": { "X": "$XP", "Y": "$YP" } } } },
                        { "inside": { "stopBy": "end", "kind": "assignment_expression", "has": { "field": "left", "has": {
                            "stopBy": "end",
                            "pattern": "$Y",
                            "compare": { "left": "Y", "op": "eq", "right": "X", "text": "index" },
                            "inside": { "pattern": "$YPC", "compare": { "left": "YPC", "op": "eq", "right": "YP", "text": "node" } },
                        } } } },
                    ] },
                ],
            } },
        },
    })
}

#[test]
fn a_pair_of_captures_recurses_element_by_element_to_any_depth() {
    let rule = compiled(self_assign());
    let source = "[a, [b, [c, [d, [e]]]]] = [a, [b, [c, [d, [e]]]]];\n\
                  [x, [y, z]] = [x, [z, y]];\n\
                  p = p;\n\
                  [q, r] = [r, q];\n";
    let result = evaluate_file(&rule, "typescript", "a.ts", source);
    assert_eq!(
        matched_texts(&result, source),
        [
            "a L=a R=a",
            "b L=b R=b",
            "c L=c R=c",
            "d L=d R=d",
            "e L=e R=e",
            "x L=x R=x",
            "p L=p R=p"
        ],
        "{result}"
    );
    // Each level's `XP`, `YP`, `Y` and `YPC` are its own: the captures of the query are the
    // rule's and the ones its calls return.
    let mut messages = self_assign();
    messages["rule"]["kind"] = json!("identifier");
    let document = check_document(json!({
        "id": "ast-match",
        "forbid": { "ast": { "language": "typescript", "rule": messages["rule"], "utils": messages["utils"] } },
        "finding": { "anchor": "witness.span", "code": "X", "message": "$L at $YP", "severity": "error" },
    }));
    assert_eq!(
        rejection(compile_document_with(document, &defaults())),
        "/checks/0/finding/message: `$YP` names no capture; the query can bind: L, R; write `\\$YP` for literal text"
    );
}

fn defaults() -> Value {
    serde_json::from_str(CAPABILITIES).expect("capabilities")
}

#[test]
fn a_call_passes_its_callers_captures_in_and_returns_what_it_binds() {
    // `same` reads both parameters, which the caller bound.
    let rule = compiled(json!({
        "rule": { "pattern": "$A = $B", "matches": { "same": { "P": "$A", "Q": "$B" } } },
        "utils": { "same": { "params": ["P", "Q"], "rule": { "compare": { "left": "P", "op": "eq", "right": "Q", "text": "tokens" } } } },
    }));
    let source = "a = a;\na = b;\n";
    let result = evaluate_file(&rule, "typescript", "a.ts", source);
    assert_eq!(
        matched_texts(&result, source),
        ["a = a A=a B=a"],
        "{result}"
    );

    // `callee` binds its parameter: the caller's `$C` holds it; `$N`, the frame's own, does not leak.
    let rule = compiled(json!({
        "rule": { "kind": "call_expression", "matches": { "callee": { "F": "$C" } } },
        "utils": { "callee": { "params": ["F"], "rule": {
            "has": { "field": "function", "pattern": "$F" },
            "not": { "has": { "field": "arguments", "has": { "pattern": "$N", "kind": "number" } } },
        } } },
    }));
    let source = "f(x);\ng(1);\n";
    let result = evaluate_file(&rule, "typescript", "a.ts", source);
    assert_eq!(matched_texts(&result, source), ["f(x) C=f"], "{result}");

    // A frame's capture named like one of its caller's is another capture.
    let rule = compiled(json!({
        "rule": { "pattern": "$L($$$)", "has": { "field": "arguments", "has": { "matches": { "arg": { "A": "$A" } } } } },
        "utils": { "arg": { "params": ["A"], "rule": { "pattern": "$A", "has": { "stopBy": "end", "pattern": "$L", "kind": "identifier" } } } },
    }));
    let source = "f(g(h));\n";
    let result = evaluate_file(&rule, "typescript", "a.ts", source);
    assert_eq!(
        matched_texts(&result, source),
        ["f(g(h)) A=g(h) L=f"],
        "{result}"
    );
}

#[test]
fn calls_take_part_in_capture_ordering_and_the_never_bound_walk() {
    // The comparison reads what a sibling relation's call returns: it runs after it.
    let rule = compiled(json!({
        "rule": {
            "kind": "call_expression",
            "has": { "field": "arguments", "has": { "pattern": "$ARG" } },
            "compare": { "left": "ARG", "op": "eq", "right": "FN", "text": "tokens" },
            "matches": { "callee": { "F": "$FN" } },
        },
        "utils": { "callee": { "params": ["F"], "rule": { "has": { "field": "function", "pattern": "$F" } } } },
    }));
    let source = "f(f);\nf(g);\n";
    let result = evaluate_file(&rule, "typescript", "a.ts", source);
    assert_eq!(
        matched_texts(&result, source),
        ["f(f) ARG=f FN=f"],
        "{result}"
    );

    // A parameterized utility reads its frame only: its parameters and its own captures.
    assert_eq!(
        rejection(compile_typescript_ast(json!({
            "rule": { "pattern": "$A($B)", "matches": { "u": { "P": "$A" } } },
            "utils": { "u": { "params": ["P"], "rule": { "compare": { "left": "P", "op": "eq", "right": "B" } } } },
        }))),
        "/checks/0/forbid/ast/utils/u/rule/compare/right: `$B` is never bound where this comparison runs (names bound under `not` or `count` are visible only inside it; a stopBy rule sees only the names it binds itself)"
    );
    // A utility it calls by name shares its frame.
    compiled(json!({
        "rule": { "pattern": "$A($B)", "matches": { "u": { "P": "$A" } } },
        "utils": {
            "u": { "params": ["P"], "rule": { "has": { "field": "arguments", "has": { "matches": "v" } } } },
            "v": { "pattern": "$Q", "compare": { "left": "P", "op": "ne", "right": "Q", "text": "tokens" } },
        },
    }));
}

#[test]
fn a_call_under_not_and_three_valued_terms() {
    // Under `not` a call binds nothing its caller keeps; an unknown verdict in the frame is unknown.
    let rule = compiled(json!({
        "rule": { "kind": "call_expression", "not": { "matches": { "unknown-callee": { "F": "$F" } } } },
        "utils": { "unknown-callee": { "params": ["F"], "rule": { "has": { "field": "function", "pattern": "$F", "testUnknown": "^u" } } } },
    }));
    let source = "u();\nf();\n";
    let result = evaluate_file(&rule, "typescript", "a.ts", source);
    assert_eq!(
        matched_texts(&result, source),
        Vec::<String>::new(),
        "{result}"
    );
    let rule = compiled(json!({
        "rule": { "kind": "call_expression", "not": { "matches": { "callee-g": { "F": "$F" } } } },
        "utils": { "callee-g": { "params": ["F"], "rule": { "has": { "field": "function", "pattern": "$F", "regex": "^g$" } } } },
    }));
    let source = "g();\nf();\n";
    let result = evaluate_file(&rule, "typescript", "a.ts", source);
    assert_eq!(matched_texts(&result, source), ["f()"], "{result}");
}

#[test]
fn each_enumerates_the_bindings_a_call_returns() {
    let document = check_document(json!({
        "id": "ast-match",
        "forbid": { "ast": {
            "language": "typescript",
            "rule": { "kind": "call_expression", "matches": { "argument": { "A": "$ARG" } } },
            "utils": { "argument": { "params": ["A"], "rule": { "has": { "field": "arguments", "has": { "kind": "identifier", "pattern": "$A" } } } } },
        } },
        "finding": { "anchor": "capture.ARG", "code": "X", "message": "$ARG", "severity": "error", "each": "ARG" },
    }));
    let rule = compile_document_with(document, &defaults())
        .unwrap_or_else(|error| panic!("{}", error.reason));
    let source = "f(a, 1, b);\n";
    let result = evaluate_file(&rule, "typescript", "a.ts", source);
    let messages = result["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|finding| finding["message"].as_str().expect("message").to_owned())
        .collect::<Vec<_>>();
    assert_eq!(messages, ["a", "b"], "{result}");
}

#[test]
fn a_define_shares_parameterized_utilities() {
    let document = json!({ "format": "rerail.policy/v1", "id": "p", "revision": 1, "checks": [
        { "id": "shared", "define": { "language": "typescript", "utils": {
            "callee": { "params": ["F"], "rule": { "has": { "field": "function", "pattern": "$F" } } },
        } } },
        { "id": "q", "forbid": { "ast": { "language": "typescript", "rule": { "kind": "call_expression", "matches": { "callee": { "F": "$C" } } } } },
          "finding": { "anchor": "capture.C", "code": "X", "message": "$C", "severity": "error" } },
    ] });
    let rule = compile_document_with(document, &defaults())
        .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&rule, "typescript", "a.ts", "f(1);\n");
    assert_eq!(result["findings"][0]["message"], "f", "{result}");
}

#[test]
fn parameterized_utilities_and_calls_are_checked_where_they_are_written() {
    let callee =
        json!({ "params": ["F"], "rule": { "has": { "field": "function", "pattern": "$F" } } });
    for (ast, reason) in [
        (
            json!({ "rule": { "matches": "callee" }, "utils": { "callee": callee } }),
            "/checks/0/forbid/ast/rule/matches: utility rule `callee` takes parameters (F); call it as { \"callee\": { \"F\": \"$NAME\" } }",
        ),
        (
            json!({ "rule": { "matches": { "callee": { "G": "$C" } } }, "utils": { "callee": callee } }),
            "/checks/0/forbid/ast/rule/matches/callee/G: `G` is not a parameter of `callee` (F)",
        ),
        (
            json!({ "rule": { "matches": { "callee": {} } }, "utils": { "callee": callee } }),
            "/checks/0/forbid/ast/rule/matches/callee: parameter `F` is not passed; map it to a capture (\"F\": \"$NAME\")",
        ),
        (
            json!({ "rule": { "matches": { "callee": { "F": "C" } } }, "utils": { "callee": callee } }),
            "/checks/0/forbid/ast/rule/matches/callee/F must name a capture: `$NAME` (A-Z, 0-9, _)",
        ),
        (
            json!({ "rule": { "matches": { "callee": { "F": "$$$C" } } }, "utils": { "callee": callee } }),
            "/checks/0/forbid/ast/rule/matches/callee/F must name a capture: `$NAME` (A-Z, 0-9, _)",
        ),
        (
            json!({ "rule": { "matches": { "callee": { "F": "$C" }, "other": {} } }, "utils": { "callee": callee } }),
            "/checks/0/forbid/ast/rule/matches must call one utility: { \"id\": { \"PARAM\": \"$CAPTURE\" } }",
        ),
        (
            json!({ "rule": { "matches": { "missing": { "F": "$C" } } } }),
            "/checks/0/forbid/ast/rule/matches: utility rule `missing` is not defined in this query's utils or a define of its language",
        ),
        (
            json!({ "rule": { "kind": "number" }, "utils": { "u": { "params": [], "rule": { "kind": "number" } } } }),
            "/checks/0/forbid/ast/utils/u/params must be a non-empty array of capture names",
        ),
        (
            json!({ "rule": { "kind": "number" }, "utils": { "u": { "params": ["a"], "rule": { "kind": "number" } } } }),
            "/checks/0/forbid/ast/utils/u/params/0 must be a capture name without `$` (A-Z, 0-9, _)",
        ),
        (
            json!({ "rule": { "kind": "number" }, "utils": { "u": { "params": ["A", "A"], "rule": { "kind": "number" } } } }),
            "/checks/0/forbid/ast/utils/u/params/1: `A` is listed twice",
        ),
        (
            json!({ "rule": { "kind": "number" }, "utils": { "u": { "params": ["A"] } } }),
            "/checks/0/forbid/ast/utils/u/rule is required",
        ),
        (
            json!({ "rule": { "kind": "number" }, "utils": { "u": { "params": ["A"], "rule": { "kind": "number" }, "kind": "number" } } }),
            "unknown field `kind` at /checks/0/forbid/ast/utils/u",
        ),
        (
            json!({ "rule": { "kind": "number" }, "utils": { "u": { "params": ["A"], "rule": { "stopBy": "end" } } } }),
            "unknown field `stopBy` at /checks/0/forbid/ast/utils/u/rule",
        ),
        (
            json!({ "rule": { "matches": { "u": { "A": "$A" } } }, "utils": { "u": { "params": ["A"], "rule": { "not": { "matches": { "u": { "A": "$A" } } } } } } }),
            "/checks/0/forbid/ast/utils/u: utility rule `u` has a cyclic dependency in its `matches` sub-rule",
        ),
    ] {
        assert_eq!(rejection(compile_typescript_ast(ast)), reason);
    }
}

#[test]
fn distinct_calls_are_capped_per_query() {
    let calls = |count: usize| {
        (0..count)
            .map(|index| json!({ "matches": { "callee": { "F": format!("$C{index}") } } }))
            .collect::<Vec<_>>()
    };
    let ast = |count: usize| {
        json!({
            "rule": { "kind": "call_expression", "any": calls(count) },
            "utils": { "callee": { "params": ["F"], "rule": { "has": { "field": "function", "pattern": "$F" } } } },
        })
    };
    compiled(ast(super::super::matcher::MAX_UTIL_INSTANCES));
    assert_eq!(
        rejection(compile_typescript_ast(ast(
            super::super::matcher::MAX_UTIL_INSTANCES + 1
        ))),
        "/checks/0/forbid/ast/rule/any/1024/matches: more than 1024 distinct parameterized utility calls in one query"
    );
}

#[test]
fn a_call_rejects_different_parameter_bindings_mapped_to_one_capture() {
    let rule = compiled(json!({
        "rule": {
            "kind": "call_expression",
            "matches": { "same_arguments": { "P": "$X", "Q": "$X" } }
        },
        "utils": {
            "same_arguments": {
                "params": ["P", "Q"],
                "rule": { "pattern": "f($P, $Q)" }
            }
        }
    }));
    let source = "f(a, a);\nf(a, b);\n";
    let result = evaluate_file(&rule, "typescript", "a.ts", source);
    assert_eq!(result["failures"], json!([]), "{result}");
    assert_eq!(matched_texts(&result, source), ["f(a, a) X=a"]);
}
