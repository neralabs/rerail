//! Three-valued matching, driven by the test-only `testUnknown` leaf: unknown where its regex
//! matches the node's text, true elsewhere.

use crate::policy_dsl::language::GrammarDialect;
use crate::policy_dsl::test_support::*;
use ast_grep_core::AstGrep;
use ast_grep_core::tree_sitter::StrDoc;
use serde_json::{Value, json};

fn texts(rule: Value, source: &str) -> Vec<String> {
    let compiled = compile_typescript_rule(rule).unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "typescript", "a.ts", source);
    assert_eq!(result["failures"], json!([]), "{result}");
    matched_texts(&result, source)
}

/// The reference matcher's matches (`GrammarMatcher::match_root` at every node), printed as
/// `matched_texts` prints findings, with the capture `A` if bound.
fn reference_texts(rule: Value, source: &str) -> Vec<String> {
    let compiled = compile_typescript_rule(rule).unwrap_or_else(|error| panic!("{}", error.reason));
    let matcher = &grammar_matchers(&compiled)[0];
    let root = AstGrep::doc(StrDoc::new(source, GrammarDialect::TypeScript));
    let mut work = usize::MAX;
    root.root()
        .dfs()
        .filter_map(|node| matcher.match_root(node, &mut work).expect("within limits"))
        .map(|found| {
            let mut parts = vec![found.text().into_owned()];
            if let Some(captured) = found.get_env().get_match("A") {
                parts.push(format!("A={}", captured.text()));
            }
            parts.join(" ")
        })
        .collect()
}

/// An identifier that is unknown when it is `u`, true when it is `x`, and false otherwise.
fn u_or_x() -> Value {
    json!({ "kind": "identifier", "regex": "^(u|x)$", "testUnknown": "^u$" })
}

#[test]
fn an_unknown_root_is_no_match_and_its_negation_is_unknown_too() {
    let source = "a; u; x;\n";
    assert_eq!(texts(u_or_x(), source), ["x"]);
    // `not` of unknown stays unknown; `not` of false holds; `not` of true fails.
    let negated = json!({ "kind": "identifier", "not": u_or_x() });
    assert_eq!(texts(negated, source), ["a"]);
    // A double negation is the original verdict.
    let double = json!({ "kind": "identifier", "not": { "not": u_or_x() } });
    assert_eq!(texts(double, source), ["x"]);
}

#[test]
fn any_holds_on_a_true_branch_and_all_fails_on_any_false_term() {
    let source = "a; u; x;\n";
    // `any`: a true branch decides; with none, an unknown branch makes the verdict unknown.
    let either = json!({ "kind": "identifier", "any": [u_or_x(), { "regex": "^a$" }] });
    assert_eq!(texts(either.clone(), source), ["a", "x"]);
    assert_eq!(
        texts(json!({ "kind": "identifier", "not": either }), source),
        Vec::<String>::new()
    );
    // `all` is false when any term is false, whichever comes first: past an unknown term that
    // binds nothing it goes on, and a later false term decides.
    let unknown_first =
        json!({ "kind": "identifier", "not": { "all": [u_or_x(), { "regex": "^x$" }] } });
    assert_eq!(texts(unknown_first, source), ["a", "u"]);
    let false_first =
        json!({ "kind": "identifier", "not": { "all": [{ "regex": "^x$" }, u_or_x()] } });
    assert_eq!(texts(false_first, source), ["a", "u"]);
    // With no false term the verdict stays unknown.
    let unknown_only =
        json!({ "kind": "identifier", "not": { "all": [u_or_x(), { "regex": "^(u|x)$" }] } });
    assert_eq!(texts(unknown_only, source), ["a"]);
    // So a relation that binds searches past a candidate an unknown term would have decided,
    // where a later term of the same rule object is false: here `f` is no identifier `u` or `x`.
    let first_call = json!({ "kind": "call_expression", "has": { "stopBy": "end", "pattern": "$A", "all": [u_or_x(), { "regex": "^(x|y)$" }] } });
    assert_eq!(texts(first_call, "f(u, x);\nf(u);\n"), ["f(u, x) A=x"]);
    // Past an unknown term that binds (here a relation whose rule binds `$A`), a later term that
    // names none of what it would have bound still decides when false.
    let binding_first = json!({ "kind": "call_expression", "not": { "all": [
        { "has": { "stopBy": "end", "pattern": "$A", "all": [u_or_x()] } },
        { "regex": "^never$" },
    ] } });
    assert_eq!(texts(binding_first, "f(u);\n"), ["f(u)"]);
}

/// A rule object's keys are an implicit `all`: false when any term is false, unknown only when no
/// term is false and some term is unknown, whatever order the terms run in. A term that reads or
/// binds a capture an unknown term would have bound is not evaluated: it stays open.
#[test]
fn implicit_conjunctions_are_false_when_any_term_is_false() {
    let source = "f(u);\nf(x);\n";
    // The binding unknown term sits in an `any`; the rule object's `any` runs before its `regex`.
    let unknown = || json!({ "has": { "stopBy": "end", "pattern": "$A", "all": [u_or_x()] } });
    let negated = |inner: Value| json!({ "kind": "call_expression", "not": inner });
    let with_any = |sibling: Value| {
        let mut inner = json!({ "any": [unknown()] });
        inner
            .as_object_mut()
            .expect("object")
            .extend(sibling.as_object().expect("sibling").clone());
        negated(inner)
    };
    let both = ["f(u)", "f(x)"];
    assert_eq!(texts(with_any(json!({ "regex": "^never$" })), source), both);
    // A false `not`, relation or `count` beside it decides too.
    assert_eq!(
        texts(
            with_any(json!({ "not": { "kind": "call_expression" } })),
            source
        ),
        both
    );
    assert_eq!(
        texts(
            with_any(json!({ "inside": { "kind": "class_body", "stopBy": "end" } })),
            source
        ),
        both
    );
    assert_eq!(
        texts(
            with_any(json!({ "count": { "has": { "kind": "arguments" }, "min": 2 } })),
            source
        ),
        both
    );
    // With no false term the verdict stays unknown: `f(u)` is open, `f(x)` holds `any`.
    assert_eq!(
        texts(with_any(json!({ "regex": "^f" })), source),
        Vec::<String>::new()
    );
    // A term that reads what the unknown term would have bound stays open, even when it would
    // fail with the capture unbound.
    let reads = json!({ "kind": "call_expression", "pattern": "$F($$$)", "not": {
        "any": [unknown()],
        "compare": { "left": "A", "op": "eq", "right": "F", "text": "source" },
    } });
    assert_eq!(texts(reads, source), ["f(x) F=f"]);
    // So is a term that binds such a capture again, and what reads that term's captures.
    let rebinds = json!({ "kind": "call_expression", "not": {
        "any": [unknown()],
        "has": { "stopBy": "end", "pattern": "$A", "regex": "^never$" },
    } });
    assert_eq!(texts(rebinds, source), ["f(x)"]);
}

/// ESLint's M10 run: a negated `inside` whose rule object holds an `any` with an unknown member
/// comparison and a false `not has`; the method has a computed key, so it cannot match.
#[test]
fn a_false_sibling_decides_a_negated_relation_beside_an_unknown_any() {
    let rule = json!({ "kind": "predefined_type", "not": { "inside": { "stopBy": "end",
        "kind": "method_definition", "not": { "has": { "kind": "computed_property_name" } },
        "pattern": "$M", "any": [{ "follows": { "stopBy": "end", "kind": "method_signature",
            "pattern": "$S", "compare": { "left": "M", "op": "eq", "right": "S", "text": "member" } } }] } } });
    assert_eq!(
        texts(rule, "class A { f(): void; [x](): string {} }\n"),
        ["void", "string"]
    );
}

#[test]
fn relations_search_past_unknown_nodes_unless_their_rule_binds() {
    let source = "f(u, x);\nf(u);\nf(y);\n";
    let calls = |has: Value| json!({ "kind": "call_expression", "has": has });
    let has = |mut rule: Value| {
        rule["stopBy"] = json!("end");
        rule
    };
    // Nothing is bound, so a later match decides: `f(u, x)` holds, `f(u)` is unknown.
    assert_eq!(texts(calls(has(u_or_x())), source), ["f(u, x)"]);
    let negated = json!({ "kind": "call_expression", "not": { "has": has(u_or_x()) } });
    assert_eq!(texts(negated, source), ["f(y)"]);
    // A binding rule is strict: the unknown `u` could be the first match, which would bind it.
    let mut binding = u_or_x();
    binding["pattern"] = json!("$A");
    assert_eq!(texts(calls(has(binding)), source), Vec::<String>::new());
    // Siblings and ancestors walk the same way.
    let before_x = json!({ "kind": "identifier", "regex": "^x$", "follows": { "stopBy": "end", "any": [u_or_x(), { "regex": "^f$" }] } });
    assert_eq!(texts(before_x, "f(u, x);\n"), Vec::<String>::new());
    let after_u = json!({ "kind": "identifier", "regex": "^y$", "not": { "follows": { "stopBy": "end", "all": [{ "kind": "identifier" }, u_or_x()] } } });
    assert_eq!(texts(after_u, "g(u, y);\ng(z, y);\n"), ["y"]);
}

#[test]
fn an_unknown_stop_verdict_ends_the_walk_with_an_open_answer() {
    let source = "function f() { z; }\n";
    let inside = |stop: Value| json!({ "kind": "identifier", "regex": "^z$", "not": { "inside": { "kind": "function_declaration", "stopBy": stop } } });
    // Stopping at the body leaves the function unreached: `not inside` holds.
    assert_eq!(
        texts(inside(json!({ "kind": "statement_block" })), source),
        ["z"]
    );
    // Whether the walk stops at the body is unknown, so whether the function is reached is too.
    assert_eq!(
        texts(
            inside(json!({ "kind": "statement_block", "testUnknown": "." })),
            source
        ),
        Vec::<String>::new()
    );
    let has = |stop: Value| json!({ "kind": "function_declaration", "not": { "has": { "kind": "identifier", "regex": "^z$", "stopBy": stop } } });
    assert_eq!(
        texts(has(json!({ "kind": "statement_block" })), source),
        ["function f() { z; }"]
    );
    assert_eq!(
        texts(
            has(json!({ "kind": "statement_block", "testUnknown": "." })),
            source
        ),
        Vec::<String>::new()
    );
}

#[test]
fn a_count_is_unknown_when_its_unknown_nodes_could_move_it_across_a_bound() {
    let source = "f(x, x);\nf(u, x);\nf(x);\nf(u, u, x);\n";
    let count = |bounds: Value| {
        let mut count = json!({ "has": { "stopBy": "end", "all": [u_or_x()] } });
        count
            .as_object_mut()
            .expect("object")
            .extend(bounds.as_object().expect("bounds").clone());
        json!({ "kind": "call_expression", "count": count })
    };
    let negated = |bounds: Value| json!({ "kind": "call_expression", "not": { "count": count(bounds)["count"] } });
    // At least two: `f(x, x)` holds, `f(x)` fails, the others are open.
    assert_eq!(texts(count(json!({ "min": 2 })), source), ["f(x, x)"]);
    assert_eq!(texts(negated(json!({ "min": 2 })), source), ["f(x)"]);
    // At most one: `f(x)` holds, `f(x, x)` fails, `f(u, x)` is open.
    assert_eq!(texts(count(json!({ "max": 1 })), source), ["f(x)"]);
    assert_eq!(texts(negated(json!({ "max": 1 })), source), ["f(x, x)"]);
    // Two or three: `f(u, u, x)` counts one to three, so it is open.
    assert_eq!(
        texts(count(json!({ "min": 2, "max": 3 })), source),
        ["f(x, x)"]
    );
}

#[test]
fn an_unknown_constraint_fails_its_rule_and_negations_stay_unknown() {
    let rule = compiled(json!({
        "rule": { "pattern": "f($A)" },
        "constraints": { "A": { "not": { "testUnknown": "^u$" } } },
    }));
    let source = "f(u);\nf(v);\n";
    let result = evaluate_file(&rule, "typescript", "a.ts", source);
    // `not` of true fails for `v`, and `not` of unknown fails for `u`.
    assert_eq!(
        matched_texts(&result, source),
        Vec::<String>::new(),
        "{result}"
    );
    let rule = compiled(json!({
        "rule": { "pattern": "f($A)" },
        "constraints": { "A": { "testUnknown": "^u$" } },
    }));
    let result = evaluate_file(&rule, "typescript", "a.ts", source);
    assert_eq!(matched_texts(&result, source), ["f(v) A=v"], "{result}");
}

#[test]
fn only_checks_that_read_values_are_three_valued() {
    let three_valued = |ast: Value| {
        grammar_matchers(&compiled(ast))
            .iter()
            .all(|grammar| grammar.three_valued)
    };
    let plain = json!({
        "rule": { "pattern": "f($A, $B)", "not": { "has": { "kind": "number" } }, "compare": { "left": "A", "op": "lt", "right": "B" } },
    });
    assert!(!three_valued(plain));
    for ast in [
        json!({ "rule": { "kind": "number", "value": { "eq": 1 } } }),
        json!({ "rule": { "pattern": "f($A)" }, "constraints": { "A": { "not": { "value": { "truthy": true } } } } }),
        json!({ "rule": { "kind": "call_expression", "matches": "one" }, "utils": { "one": { "has": { "stopBy": "end", "value": { "eq": 1 } } } } }),
        json!({ "rule": { "pattern": "f($A, $B)", "compare": { "left": "A", "op": "eq", "right": "B", "text": "key" } } }),
        json!({ "rule": { "kind": "number", "inside": { "kind": "program", "stopBy": { "value": { "eq": 1 } } } } }),
    ] {
        assert!(three_valued(ast.clone()), "{ast}");
    }
    // Tokens are always known.
    assert!(!three_valued(json!({
        "rule": { "pattern": "f($A, $B)", "compare": { "left": "A", "op": "eq", "right": "B", "text": "tokens" } },
    })));
}

/// M12b's relations and positions are three-valued like the rest: a `has` or `inside` narrowed by
/// `field` tests only that field's children, and an `nthChild` whose `ofRule` is unknown on a
/// sibling has an unknown position, so a negation of either stays unknown there.
#[test]
fn field_and_nth_child_stay_unknown_under_not() {
    let source = "a = u;\na = x;\na = b;\nu = b;\n";
    let right = |rule: Value| json!({ "field": "right", "all": [rule] });
    // `u = b`'s unknown `u` is its left side, which the field excludes.
    let negated = json!({ "kind": "assignment_expression", "not": { "has": right(u_or_x()) } });
    assert_eq!(texts(negated, source), ["a = b", "u = b"]);
    let unnarrowed = json!({ "kind": "assignment_expression", "not": { "has": u_or_x() } });
    assert_eq!(texts(unnarrowed, source), ["a = b"]);
    let left_of = json!({ "kind": "identifier", "regex": "^(a|u)$", "not": { "inside": {
        "field": "left", "kind": "assignment_expression", "has": right(u_or_x()) } } });
    assert_eq!(texts(left_of, source), ["u", "a", "u"]);
    // A false sibling decides past a `field` relation that is unknown and binds.
    let decided = json!({ "kind": "assignment_expression", "not": {
        "has": { "field": "right", "pattern": "$A", "all": [u_or_x()] }, "regex": "^never$" } });
    assert_eq!(texts(decided, source).len(), 4);
    // An unknown sibling under `ofRule` leaves every position among those siblings unknown.
    let source = "f(u, a);\nf(x, a);\nf(a, b);\n";
    let first = json!({ "kind": "identifier", "inside": { "kind": "arguments" },
        "not": { "nthChild": { "position": 1, "ofRule": u_or_x() } } });
    assert_eq!(texts(first, source), ["a", "a", "b"]);
    let plain = json!({ "kind": "identifier", "inside": { "kind": "arguments" }, "not": { "nthChild": 1 } });
    assert_eq!(texts(plain, source), ["a", "a", "b"]);
}

/// A comparison of a transform variable mentions the captures it is computed from: past an unknown
/// term that would have bound them it stays open, where reading the variable unbound would have
/// decided the conjunction false.
#[test]
fn a_transform_comparison_waits_for_the_captures_it_reads() {
    let compiled = compile_typescript_ast(json!({
        // The `any` lets the query bind `$A` outside the `not`, as a transform's source must.
        "rule": { "pattern": "$F($$$)", "any": [
            { "has": { "stopBy": "end", "pattern": "$A", "regex": "^never$" } },
            { "kind": "call_expression" },
        ], "not": {
            "has": { "stopBy": "end", "pattern": "$A", "all": [u_or_x()] },
            "compare": { "left": "T", "op": "eq", "right": "F", "text": "source" },
        } },
        "transform": { "T": { "convert": { "source": "$A", "toCase": "upperCase" } } },
    }))
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let source = "X(u);\nX(x);\nX(a);\n";
    let result = evaluate_file(&compiled, "typescript", "a.ts", source);
    assert_eq!(result["failures"], json!([]), "{result}");
    assert_eq!(matched_texts(&result, source), ["X(a) F=X"]);
}

/// An `any` branch whose verdict is unknown and that could bind ends the search unknown, as a
/// strict relation search does: a later branch must not substitute its captures for the ones the
/// unknown branch would have bound (SPEC-S4). An unknown branch that binds nothing lets a later
/// branch decide.
#[test]
fn a_later_any_branch_does_not_stand_in_for_an_unknown_binding_branch() {
    let source = "f(u);\n";
    let any = |first: Value| {
        json!({ "kind": "call_expression", "any": [
            { "has": first },
            { "has": { "stopBy": "end", "pattern": "$A", "kind": "identifier" } },
        ] })
    };
    let mut first = json!({ "stopBy": "end", "pattern": "$A", "all": [u_or_x()] });
    assert_eq!(
        reference_texts(any(first.clone()), source),
        Vec::<String>::new()
    );
    first.as_object_mut().expect("object").remove("pattern");
    assert_eq!(reference_texts(any(first), source), ["f(u) A=f"]);
}

/// An occurrence search reads like a relation's: a later occurrence that matches decides past an
/// unknown one, and leaves no unknown verdict behind for the terms after it; a binding rule is
/// strict, ending the search unknown at the unknown occurrence.
#[test]
fn an_occurrence_search_decides_past_unknown_occurrences_unless_its_rule_binds() {
    let source = "let v = 0;\ng(v);\nh(v);\n";
    let declared = |rule: Value| json!({ "kind": "identifier", "inside": { "kind": "variable_declarator" }, "all": [rule] });
    let binds = |mut occurrence: Value| {
        occurrence["inside"] = json!({ "stopBy": "end", "kind": "call_expression", "regex": "^(g|h)\\(", "testUnknown": "^g\\(" });
        json!({ "binds": { "occurrence": occurrence } })
    };
    assert_eq!(texts(declared(binds(json!({}))), source), ["v"]);
    let false_beside = |occurrence: Value| {
        declared(json!({ "not": { "all": [binds(occurrence), { "regex": "^never$" }] } }))
    };
    assert_eq!(texts(false_beside(json!({})), source), ["v"]);
    let binding = json!({ "pattern": "$A" });
    assert_eq!(
        texts(declared(binds(binding.clone())), source),
        Vec::<String>::new()
    );
    assert_eq!(texts(false_beside(binding), source), ["v"]);
}

/// An `any` whose branch is unknown where its `binds` holds stays unknown there, and is no match
/// only where the `binds` names no binding. Under `not`, an unknown wrongly read as no match
/// would be reported.
#[test]
fn a_binds_branch_never_turns_an_unknown_into_no_match() {
    let check = json!({
        "id": "binds",
        "forbid": { "ast": { "language": "python", "rule": {
            "kind": "identifier",
            "pattern": "$N",
            "not": { "any": [
                { "testUnknown": "^u", "binds": { "form": "assignment" } },
                { "binds": { "form": "import" } },
            ] },
        } } },
        "violation": { "anchor": "capture.N", "code": "G", "message": "g" },
    });
    let compiled = compile_document_with(
        check_document(check),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let source = "u1 = 1\nu2\nimport os\nos\nx = u1\n";
    let result = evaluate_file(&compiled, "python", "binds.py", source);
    assert_eq!(result["failures"], json!([]), "{result}");
    let mut starts = result["violations"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|finding| finding["span"]["startByte"].as_u64().expect("start"))
        .collect::<Vec<_>>();
    starts.sort_unstable();
    // `u1`'s binding is unknown (no finding); `x`'s binding matches; the import binds its
    // `dotted_name`, not the `os` identifier inside it; that identifier and the references `u2`,
    // `os` and `u1` name no binding, so the `any` is no match there and `not` reports them.
    let at = |needle: &str, from: usize| (from + source[from..].find(needle).expect(needle)) as u64;
    let expected = vec![at("u2", 0), at("os", 10), at("os\n", 18), at("u1", 10)];
    assert_eq!(starts, expected, "{result}");
}
