//! The hard cases of the design (`docs/sql-set-composition-design.md`, "Known hard cases") in
//! scope for gate G1, one test each: both engines must report the same findings, and each test
//! also pins the native answer so it exercises the case it names.

use super::{evaluate, same, snapshot};
use crate::code_rule_dsl::CompiledCodeRuleDsl;
use crate::code_rule_dsl::test_support::{CAPABILITIES, clause_document, compile_document_with};
use serde_json::{Value, json};

/// One clause in `language` with `ast` (`rule`, `utils`, `constraints`).
pub(super) fn compile(language: &str, mut ast: Value) -> CompiledCodeRuleDsl {
    ast["language"] = json!(language);
    compile_document_with(
        clause_document(json!({
            "id": "ast-match",
            "forbid": { "ast": ast },
            "violation": { "anchor": "witness.span", "code": "X", "message": "x" },
        })),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason))
}

/// Each violation's text and its captures' texts, `TEXT NAME=text…` (a list's nodes joined by
/// `|`).
pub(super) fn texts(result: &Value, source: &str) -> Vec<String> {
    let text = |span: &Value| {
        let start = span["startByte"].as_u64().expect("start") as usize;
        let end = span["endByte"].as_u64().expect("end") as usize;
        &source[start..end]
    };
    result["violations"]
        .as_array()
        .expect("violations")
        .iter()
        .map(|violation| {
            let mut parts = vec![text(&violation["span"]).to_owned()];
            for (name, spans) in violation["captures"].as_object().expect("captures") {
                let joined = spans
                    .as_array()
                    .expect("spans")
                    .iter()
                    .map(text)
                    .collect::<Vec<_>>()
                    .join("|");
                parts.push(format!("{name}={joined}"));
            }
            parts.join(" ")
        })
        .collect()
}

/// Both engines' findings for `ast` over `source`, which must agree; their texts.
pub(super) fn agreed(language: &str, ast: Value, source: &str) -> Vec<String> {
    let extension = match language {
        "python" => "py",
        "php" => "php",
        _ => "ts",
    };
    let compiled = compile(language, ast);
    let result = same(
        &compiled,
        &snapshot(&[(language, &format!("a.{extension}"), source)]),
    );
    assert_eq!(result["failures"], json!([]), "{result}");
    texts(&result, source)
}

#[test]
fn a_relation_binds_its_first_match_even_where_a_later_term_rejects_it() {
    // `has` binds `$X` to the first identifier argument; the later term reads `$X` and fails for
    // `f(a, b)` (its `b` is not `a`), and no other candidate is tried.
    let ast = json!({ "rule": {
        "kind": "call_expression",
        "all": [
            { "has": { "kind": "arguments", "has": { "pattern": "$X", "kind": "identifier" } } },
            { "has": { "stopBy": "end", "kind": "identifier", "regex": "^b$", "pattern": "$X" } },
        ],
    } });
    let source = "f(a, b);\ng(b, a);\n";
    assert_eq!(agreed("typescript", ast, source), ["g(b, a) X=b"]);
}

#[test]
fn any_commits_its_first_matching_branch_though_a_later_one_would_satisfy_what_follows() {
    let ast = json!({ "rule": {
        "kind": "call_expression",
        "all": [
            { "any": [
                { "has": { "kind": "arguments", "has": { "pattern": "$X", "kind": "identifier" } } },
                { "has": { "kind": "arguments", "has": { "pattern": "$X", "kind": "number" } } },
            ] },
            { "has": { "stopBy": "end", "kind": "number", "pattern": "$X" } },
        ],
    } });
    let source = "f(a, 1);\ng(2);\n";
    // `f` binds `$X = a` in the first branch; `1` is never tried. `g` falls through to the second.
    assert_eq!(agreed("typescript", ast, source), ["g(2) X=2"]);
}

#[test]
fn not_over_an_unbound_capture_matches_freely_and_over_a_bound_one_unifies() {
    let unbound = json!({ "rule": {
        "kind": "call_expression",
        "not": { "has": { "kind": "arguments", "has": { "pattern": "$X" } } },
    } });
    let source = "f();\ng(a);\nh(a, a);\n";
    assert_eq!(agreed("typescript", unbound, source), ["f()"]);
    // Bound first: `not` reads `$X` (the callee) and only an argument equal to it rejects.
    let bound = json!({ "rule": {
        "pattern": "$X($$$)",
        "not": { "has": { "kind": "arguments", "has": { "pattern": "$X" } } },
    } });
    let source = "f(f);\ng(a);\n";
    assert_eq!(agreed("typescript", bound, source), ["g(a) X=g"]);
}

#[test]
fn zero_width_siblings_follow_the_tapes_byte_positioned_walks() {
    // A missing token is a zero-width sibling: `next()` skips it, and `next_all` from it skips
    // its anchor. `nodes::every_axis_lists_the_nodes_the_tape_walks_visit` checks each walk node by
    // node over such tapes; here the relations over them agree end to end, in files with syntax
    // errors (whose clauses run around the error regions).
    for source in [
        "g(x);\nh(y, z);\nlet q = [1, 2;\nlet r = 3;\n",
        "if (a { b(); }\nc(\nd(e);\n",
    ] {
        for relation in ["precedes", "follows", "inside", "has"] {
            for stop in ["neighbor", "end"] {
                let ast = json!({ "rule": {
                    "regex": "^[a-z;,(]",
                    relation: { "stopBy": stop, "pattern": "$A" },
                } });
                let compiled = compile("typescript", ast);
                let input = snapshot(&[("typescript", "a.ts", source)]);
                assert_eq!(
                    evaluate(&compiled, &input, "native"),
                    evaluate(&compiled, &input, "sql"),
                    "{relation} {stop} over {source:?}"
                );
            }
        }
    }
}

#[test]
fn has_neighbor_walks_anonymous_children_too() {
    let ast = json!({ "rule": { "kind": "arguments", "has": { "regex": "^,$" } } });
    let source = "f(a);\ng(a, b);\n";
    assert_eq!(agreed("typescript", ast, source), ["(a, b)"]);
    let ast = json!({ "rule": {
        "kind": "call_expression",
        "has": { "kind": "arguments", "has": { "regex": "^\\($", "precedes": { "pattern": "$A" } } },
    } });
    assert_eq!(
        agreed("typescript", ast, source),
        ["f(a) A=a", "g(a, b) A=a"]
    );
}

#[test]
fn a_field_has_with_a_stop_rule_tests_one_level_below_the_field_child() {
    // The field child, then only its children: `c` two levels down is not reached.
    let ast = json!({ "rule": {
        "kind": "call_expression",
        "has": { "field": "arguments", "stopBy": { "kind": "string" }, "kind": "identifier", "pattern": "$A" },
    } });
    let source = "f(a);\ng([c]);\n";
    assert_eq!(agreed("typescript", ast, source), ["f(a) A=a"]);
    // `end` descends all the way.
    let ast = json!({ "rule": {
        "kind": "call_expression",
        "has": { "field": "arguments", "stopBy": "end", "kind": "identifier", "pattern": "$A" },
    } });
    assert_eq!(
        agreed("typescript", ast, source),
        ["f(a) A=a", "g([c]) A=c"]
    );
}

#[test]
fn list_captures_unify_over_their_named_nodes_only() {
    // `$$$A` binds the arguments with their commas; unifying a second list compares the named
    // nodes alone, so `f(a, b,)` (a trailing comma) and `g(a, b)` agree.
    let ast = json!({ "rule": {
        "kind": "expression_statement",
        "has": { "pattern": "f($$$A)" },
        "precedes": { "has": { "pattern": "g($$$A)" } },
    } });
    let source = "f(a, b,);\ng(a, b);\nf(a);\ng(a, a);\n";
    // A finding renders a list's named nodes only.
    assert_eq!(agreed("typescript", ast, source), ["f(a, b,); A=a|b"]);
    // An empty list binds (and renders no node); a call with arguments does not unify with it.
    let ast = json!({ "rule": {
        "kind": "expression_statement",
        "has": { "pattern": "f($$$A)" },
        "follows": { "has": { "pattern": "f($$$A)" } },
    } });
    let source = "f();\nf();\nf(a);\n";
    assert_eq!(agreed("typescript", ast, source), ["f(); A="]);
}

#[test]
fn every_out_of_scope_operator_is_refused_by_name() {
    let refused = |ast: Value, operator: &str| {
        let compiled = compile("typescript", ast);
        let input = snapshot(&[("typescript", "a.ts", "f(a);\n")]);
        let error = compiled
            .evaluate(input.to_string(), "report".into(), Some("sql".into()))
            .expect_err("the SQL engine refuses the clause");
        assert!(
            error.reason.contains(&format!("Unsupported({operator})")),
            "{operator}: {}",
            error.reason
        );
    };
    // A stop rule is a per-batch stage over the batch's files: a parsed tree's nodes are none.
    refused(
        json!({ "rule": { "kind": "comment", "parse": {
            "grammar": "eslintDirective",
            "has": { "stopBy": { "kind": "item" }, "kind": "item" },
        } } }),
        "stopBy rule inside parse",
    );
    // A recursion that reads its own answer at the node it started at (its parent's children
    // include it) loops: the matcher recurses until its depth limit.
    refused(
        json!({
            "rule": { "matches": "deep" },
            "utils": { "deep": { "any": [{ "kind": "number" }, { "inside": { "has": { "matches": "deep" } } }] } },
        }),
        "recursion whose reads loop back",
    );
    // The matcher leaks `ofRule`'s bindings from a candidate whose position fails to the later
    // candidates of the walk: refused, not approximated.
    refused(
        json!({ "rule": {
            "kind": "arguments",
            "has": { "nthChild": { "position": 2, "ofRule": { "pattern": "$A" } } },
        } }),
        "nthChild with a binding ofRule as a relation's rule",
    );
}
