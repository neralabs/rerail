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
fn texts(result: &Value, source: &str) -> Vec<String> {
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
fn a_descendant_search_handles_early_late_and_missing_matches() {
    let ast = json!({ "rule": {
        "kind": "expression_statement",
        "has": { "stopBy": "end", "kind": "identifier", "regex": "^b$", "pattern": "$X" }
    } });
    assert_eq!(
        agreed("typescript", ast, "f(a, b);\ng(b, a);\nh(c);\nb();\n"),
        ["f(a, b); X=b", "g(b, a); X=b", "b(); X=b"]
    );
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

/// The walks with a rule stop read the stop rule's matches through `nearest` (nearest ancestor and
/// siblings in the set), never a recursive CTE, and agree with the matcher: every relation, with a
/// field, two- and three-valued (an unknown stop leaves the walk open), around zero-width
/// siblings.
#[test]
fn rule_stop_walks_read_the_indexed_stop_set() {
    use crate::code_rule_dsl::test_support::grammar_matchers;
    let sources = [
        "g(x);\nh(y, z);\nlet q = [1, 2;\nlet r = 3;\n",
        "if (a { b(); }\nc(\nd(e);\n",
        "function f(a) { if (a) { return g(a, [b, c]); } return h(u1, (d)); }\nlet k = f(u2) + m(n);\n",
    ];
    let stops = [
        json!({ "kind": "call_expression" }),
        json!({ "any": [{ "kind": "arguments" }, { "kind": "statement_block" }] }),
        json!({ "regex": "^[a-z(]" }),
        json!({ "kind": "call_expression", "testUnknown": "^u" }),
    ];
    let mut statements = 0;
    for source in sources {
        for relation in ["precedes", "follows", "inside", "has"] {
            for (field, stop) in [None, Some("arguments"), Some("body")]
                .into_iter()
                .flat_map(|field| stops.iter().map(move |stop| (field, stop)))
            {
                if field.is_some() && !matches!(relation, "inside" | "has") {
                    continue;
                }
                let mut inner = json!({ "stopBy": stop, "pattern": "$A" });
                if let Some(field) = field {
                    inner["field"] = json!(field);
                }
                let ast = json!({ "rule": { "regex": "^[a-z;,({]", relation: inner } });
                let compiled = compile("typescript", ast);
                let input = snapshot(&[("typescript", "a.ts", source)]);
                assert_eq!(
                    evaluate(&compiled, &input, "native"),
                    evaluate(&compiled, &input, "sql"),
                    "{relation} {field:?} {stop} over {source:?}"
                );
                let grammars = grammar_matchers(&compiled);
                let program =
                    crate::code_rule_dsl::sql::compile(&[&grammars[0]]).expect("compiles");
                let mut session = program.session().expect("a session");
                let plans = session.plans().expect("plans");
                // `has` with a field and a rule stop tests one level below the field child: no
                // walk (`a_field_has_with_a_stop_rule_tests_one_level_below_the_field_child`).
                assert!(
                    (relation == "has" && field.is_some())
                        || plans.iter().any(|(sql, _)| sql.contains("nearest")),
                    "{relation} {field:?} {stop}: no walk reads `nearest`"
                );
                statements += plans.len();
            }
        }
    }
    assert!(statements > 0);
}

#[test]
fn blade_overlapping_siblings_scan_the_parents_children() {
    // A Blade tape's embedded PHP sits among the template's nodes; its sibling walks scan.
    let source = "<div>{{ $a }}</div>\n@if ($b)\n  <p>{{ $c }}</p>\n@endif\n";
    for relation in ["precedes", "follows"] {
        for stop in ["neighbor", "end"] {
            let document = clause_document(json!({
                "id": "ast-match",
                "forbid": { "ast": { "language": "blade", "rule": {
                    "regex": "^.",
                    relation: { "stopBy": stop, "regex": "^." },
                } } },
                "violation": { "anchor": "witness.span", "code": "X", "message": "x" },
            }));
            let compiled = compile_document_with(
                document,
                &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
            )
            .unwrap_or_else(|error| panic!("{}", error.reason));
            let result = same(&compiled, &snapshot(&[("blade", "a.blade.php", source)]));
            assert!(
                !result["violations"]
                    .as_array()
                    .expect("violations")
                    .is_empty(),
                "{relation} {stop}: {result}"
            );
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
fn formerly_refused_operators_use_native_fragment_semantics() {
    let source = "// eslint-disable foo, bar\nf(a, b);\n";
    for ast in [
        json!({ "rule": { "kind": "comment", "parse": {
            "grammar": "eslintDirective",
            "has": { "stopBy": { "kind": "item" }, "kind": "item" },
        } } }),
        json!({ "rule": {
            "kind": "arguments",
            "has": { "nthChild": { "position": 2, "ofRule": { "pattern": "$A" } } },
        } }),
    ] {
        // Both success and failure preserve the native binding semantics.
        agreed("typescript", ast, source);
    }
    // This recursion revisits its caller. It must return a typed depth failure, never
    // compile an SQL program counter or reject the grammar during compilation.
    let compiled = compile(
        "typescript",
        json!({
            "rule": { "matches": "deep" },
            "utils": { "deep": { "any": [{ "kind": "number" }, { "inside": { "has": { "matches": "deep" } } }] } },
        }),
    );
    let result = same(&compiled, &snapshot(&[("typescript", "a.ts", "f(a);\n")]));
    assert!(
        !result["failures"].as_array().unwrap().is_empty(),
        "{result}"
    );
}
