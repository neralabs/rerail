//! Recursive utilities and `violation.each`, one named test per shape: both engines must report
//! the same findings, and each test also pins the native answer so it exercises what it names.

use super::{same, snapshot};
use crate::code_rule_dsl::CompiledCodeRuleDsl;
use crate::code_rule_dsl::test_support::{CAPABILITIES, clause_document, compile_document_with};
use serde_json::{Value, json};

/// One clause in `language` with `ast` (`rule`, `utils`, `constraints`), enumerating `each`.
fn compile(language: &str, mut ast: Value, each: Option<&str>) -> CompiledCodeRuleDsl {
    ast["language"] = json!(language);
    let mut violation = json!({ "anchor": "witness.span", "code": "X", "message": "x" });
    if let Some(each) = each {
        violation["each"] = json!(each);
    }
    compile_document_with(
        clause_document(
            json!({ "id": "ast-match", "forbid": { "ast": ast }, "violation": violation }),
        ),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason))
}

fn extension(language: &str) -> &'static str {
    match language {
        "python" => "py",
        "php" => "php",
        _ => "ts",
    }
}

/// Each violation's text and its captures' texts, `TEXT NAME=text…`.
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
fn agreed(language: &str, ast: Value, each: Option<&str>, source: &str) -> Vec<String> {
    let compiled = compile(language, ast, each);
    let result = same(
        &compiled,
        &snapshot(&[(language, &format!("a.{}", extension(language)), source)]),
    );
    assert_eq!(result["failures"], json!([]), "{result}");
    let mut found = texts(&result, source);
    found.sort();
    found
}

/// The SQL engine's refusal of `ast` over `source`.
fn refused(language: &str, ast: Value, source: &str) -> String {
    let compiled = compile(language, ast, None);
    let input = snapshot(&[(language, &format!("a.{}", extension(language)), source)]);
    compiled
        .evaluate(
            input.to_string(),
            "report".to_owned(),
            Some("sql".to_owned()),
        )
        .expect_err("the SQL engine refuses")
        .reason
}

#[test]
fn a_recursive_utility_descends_through_has() {
    // `tail` holds where a member or call chain ends at the identifier `x`, as deep as it goes.
    let ast = json!({
        "rule": { "kind": "call_expression", "matches": "tail" },
        "utils": { "tail": { "any": [
            { "kind": "identifier", "regex": "^x$" },
            { "kind": "member_expression", "has": { "field": "object", "matches": "tail" } },
            { "kind": "call_expression", "has": { "field": "function", "matches": "tail" } },
        ] } },
    });
    let source = "x.a.b();\ny.a();\nx();\nx.c().d();\n";
    assert_eq!(
        agreed("typescript", ast, None, source),
        ["x()", "x.a.b()", "x.c()", "x.c().d()"]
    );
}

#[test]
fn a_recursive_utility_ascends_through_inside() {
    let ast = json!({
        "rule": { "kind": "identifier", "regex": "^v$", "matches": "looped" },
        "utils": { "looped": { "any": [
            { "kind": "for_statement" },
            { "inside": { "matches": "looped" } },
        ] } },
    });
    let source = "for (;;) { f(v); }\ng(v);\n";
    assert_eq!(agreed("typescript", ast, None, source), ["v"]);
}

#[test]
fn a_recursive_utility_walks_siblings() {
    // A statement some later sibling of which returns, one sibling at a time.
    let ast = json!({
        "rule": { "kind": "expression_statement", "matches": "later" },
        "utils": { "later": { "any": [
            { "kind": "return_statement" },
            { "precedes": { "matches": "later" } },
        ] } },
    });
    let source = "function f() { a(); b(); return 1; }\nfunction g() { c(); }\n";
    assert_eq!(agreed("typescript", ast, None, source), ["a();", "b();"]);
    // And backwards, through `follows`.
    let ast = json!({
        "rule": { "kind": "expression_statement", "matches": "earlier" },
        "utils": { "earlier": { "any": [
            { "kind": "lexical_declaration" },
            { "follows": { "matches": "earlier" } },
        ] } },
    });
    let source = "function f() { a(); const x = 1; b(); c(); }\n";
    assert_eq!(agreed("typescript", ast, None, source), ["b();", "c();"]);
}

#[test]
fn negation_over_a_recursive_reference_is_stratified() {
    // A block every statement of which ends in a `return`, recursively through nested blocks.
    let ast = json!({
        "rule": { "kind": "statement_block", "matches": "returns" },
        "utils": { "returns": { "any": [
            { "kind": "return_statement" },
            { "kind": "statement_block", "has": { "kind": "return_statement" },
              "not": { "has": { "kind": "if_statement", "not": { "has": { "field": "consequence", "matches": "returns" } } } } },
        ] } },
    });
    let source = "function f() { if (a) { return 1; } return 2; }\n\
                  function g() { if (a) { b(); } return 2; }\n";
    let found = agreed("typescript", ast, None, source);
    assert_eq!(
        found,
        ["{ if (a) { return 1; } return 2; }", "{ return 1; }"]
    );
    // A count over a recursive reference.
    let ast = json!({
        "rule": { "kind": "statement_block", "count": { "has": { "matches": "deep", "stopBy": "end" }, "min": 2, "as": "N" } },
        "utils": { "deep": { "any": [
            { "kind": "return_statement" },
            { "kind": "if_statement", "has": { "field": "consequence", "matches": "deep" } },
            { "kind": "statement_block", "has": { "matches": "deep" } },
        ] } },
    });
    let found = agreed("typescript", ast, None, source);
    assert!(!found.is_empty());
}

#[test]
fn a_recursive_utility_binds_captures_in_its_frames() {
    // The innermost object of a member chain, bound by the recursion's deepest level.
    let ast = json!({
        "rule": { "kind": "call_expression", "has": { "field": "function", "matches": "base" } },
        "utils": { "base": { "any": [
            { "kind": "identifier", "pattern": "$A" },
            { "kind": "member_expression", "has": { "field": "object", "matches": "base" } },
        ] } },
    });
    let source = "x.a.b();\ny();\n";
    assert_eq!(
        agreed("typescript", ast, None, source),
        ["x.a.b() A=x", "y() A=y"]
    );
}

#[test]
fn a_recursive_utility_reads_a_capture_bound_before_it() {
    // A call whose arguments hold, at any depth, its own callee.
    let ast = json!({
        "rule": { "pattern": "$A($$$)", "has": { "field": "arguments", "matches": "same" } },
        "utils": { "same": { "any": [
            { "kind": "identifier", "pattern": "$A" },
            { "has": { "matches": "same" } },
        ] } },
    });
    let source = "f(g(f));\nh(1);\nk([k]);\n";
    assert_eq!(
        agreed("typescript", ast, None, source),
        ["f(g(f)) A=f", "k([k]) A=k"]
    );
}

#[test]
fn a_recursive_parameterized_call_returns_through_its_frames() {
    let ast = json!({
        "rule": { "pattern": "$X($$$)", "has": { "field": "arguments", "matches": { "same": { "P": "$X" } } } },
        "utils": { "same": { "params": ["P"], "rule": { "any": [
            { "kind": "identifier", "pattern": "$P" },
            { "has": { "matches": { "same": { "P": "$P" } } } },
        ] } } },
    });
    let source = "f(g(f));\nh(1);\nk([k]);\n";
    assert_eq!(
        agreed("typescript", ast, None, source),
        ["f(g(f)) X=f", "k([k]) X=k"]
    );
    // Calls that recurse through each other, binding through either.
    for any in crate::code_rule_dsl::test_support::recursive_call_branch_orders() {
        let ast = json!({
            "rule": { "kind": "expression_statement", "any": any },
            "utils": crate::code_rule_dsl::test_support::mutually_recursive_calls(),
        });
        let source = "a;\nb;\n{ c; }\n";
        agreed("typescript", ast, None, source);
    }
}

#[test]
fn mutually_recursive_utilities_meet_at_the_same_node() {
    // The data-layer policies' raising-branch utilities, in small: an `if` raises where its
    // consequence and every `elif`/`else` raise.
    let ast = json!({
        "rule": { "kind": "function_definition", "has": { "field": "body", "matches": "block-raises" } },
        "utils": {
            "block-raises": { "has": { "stopBy": "neighbor", "matches": "stmt-raises" } },
            "stmt-raises": { "any": [
                { "kind": "raise_statement" },
                { "kind": "if_statement", "matches": "if-raises" },
            ] },
            "if-raises": { "all": [
                { "has": { "field": "consequence", "matches": "block-raises" } },
                { "any": [
                    { "kind": "if_statement", "has": { "stopBy": "neighbor",
                        "any": [{ "kind": "elif_clause" }, { "kind": "else_clause" }],
                        "not": { "follows": { "stopBy": "end", "any": [{ "kind": "elif_clause" }, { "kind": "else_clause" }] } },
                        "matches": "orelse-raises" } },
                    { "kind": "elif_clause", "precedes": { "stopBy": "end",
                        "any": [{ "kind": "elif_clause" }, { "kind": "else_clause" }],
                        "matches": "orelse-raises" } },
                ] },
            ] },
            "orelse-raises": { "any": [
                { "kind": "elif_clause", "matches": "if-raises" },
                { "kind": "else_clause", "has": { "field": "body", "matches": "block-raises" } },
            ] },
        },
    });
    let source = "def f():\n    if a:\n        raise X\n    else:\n        raise Y\n\
                  def g():\n    if a:\n        raise X\n    elif b:\n        pass\n    else:\n        raise Y\n\
                  def h():\n    raise Z\n";
    let found = agreed("python", ast, None, source);
    assert_eq!(found.len(), 2, "{found:?}");
}

#[test]
fn a_recursion_whose_reads_loop_back_is_refused() {
    // From a node to its parent and back to its children: the node itself again.
    let ast = json!({
        "rule": { "kind": "identifier", "matches": "loop" },
        "utils": { "loop": { "any": [
            { "kind": "number" },
            { "inside": { "has": { "matches": "loop" } } },
        ] } },
    });
    let reason = refused("typescript", ast, "f(a, 1);\n");
    assert!(
        reason.contains("Unsupported(recursion whose reads loop back)"),
        "{reason}"
    );
}

#[test]
fn a_capture_free_recursive_utility_is_one_stage_shared_by_its_clauses() {
    use crate::code_rule_dsl::test_support::{compiled, grammar_matchers};
    let utils = json!({ "tail": { "any": [
        { "kind": "identifier", "regex": "^x$" },
        { "kind": "member_expression", "has": { "field": "object", "matches": "tail" } },
    ] } });
    let rules = [
        json!({ "rule": { "kind": "call_expression", "has": { "field": "function", "matches": "tail" } }, "utils": utils.clone() }),
        json!({ "rule": { "kind": "assignment_expression", "has": { "field": "left", "matches": "tail" } }, "utils": utils }),
    ];
    let compiled: Vec<CompiledCodeRuleDsl> = rules.into_iter().map(compiled).collect();
    let grammars: Vec<_> = compiled
        .iter()
        .map(|rule| &grammar_matchers(rule)[0])
        .collect();
    let program = crate::code_rule_dsl::sql::compile(&grammars).expect("compiles");
    let (stages, _) = program.statement_count();
    let single: Vec<_> = grammars[..1].to_vec();
    let (alone, _) = crate::code_rule_dsl::sql::compile(&single)
        .expect("compiles")
        .statement_count();
    assert_eq!(
        stages, alone,
        "the second clause reuses the first one's stage"
    );
}

/// A recursive utility of a three-valued clause returns its body's unknown verdicts through
/// `matches` (`testUnknown` is unknown where its regex matches): `u.a()` is neither matched nor
/// matched by the negation, both in a capture-free cycle (a three-valued per-batch stage) and in
/// one whose frames bind a capture (a unit whose answers carry `unk`).
#[test]
fn a_three_valued_recursive_utility_returns_unknown_verdicts() {
    use crate::code_rule_dsl::sql::terminals::calls::UNKNOWN_VERDICTS;
    use crate::code_rule_dsl::test_support::grammar_matchers;
    let source = "x.a();\nu.a();\ny.a();\nx.b.c();\nu.b.c();\n";
    for (leaf, held_found) in [
        (
            json!({ "kind": "identifier", "regex": "^[xu]$", "testUnknown": "^u" }),
            ["x.a()", "x.b.c()"],
        ),
        (
            json!({ "kind": "identifier", "regex": "^[xu]$", "pattern": "$X", "testUnknown": "^u" }),
            ["x.a() X=x", "x.b.c() X=x"],
        ),
    ] {
        let utils = json!({ "tail": { "any": [
            leaf,
            { "kind": "member_expression", "has": { "field": "object", "matches": "tail" } },
            { "kind": "call_expression", "has": { "field": "function", "matches": "tail" } },
        ] } });
        let held = json!({ "rule": { "kind": "call_expression", "matches": "tail" }, "utils": utils.clone() });
        assert!(grammar_matchers(&compile("typescript", held.clone(), None))[0].three_valued);
        UNKNOWN_VERDICTS.set(0);
        assert_eq!(agreed("typescript", held, None, source), held_found);
        assert!(UNKNOWN_VERDICTS.get() > 0, "a terminal answered unknown");
        let negated = json!({
            "rule": { "kind": "call_expression", "not": { "matches": "tail" } },
            "utils": utils,
        });
        assert_eq!(agreed("typescript", negated, None, source), ["y.a()"]);
    }
}

/// `violation.each` over a three-valued clause is refused by name: its exclusion rounds skip
/// candidates in the two-valued operators only, not in `AllOpen`, `AnyOpen` or `Search`.
#[test]
fn three_valued_violation_each_is_refused() {
    let ast = json!({ "rule": {
        "kind": "call_expression",
        "has": { "field": "arguments", "has": { "kind": "identifier", "pattern": "$A", "testUnknown": "^u" } },
    } });
    let compiled = compile("typescript", ast, Some("A"));
    let input = snapshot(&[("typescript", "a.ts", "f(a, u);\n")]);
    let reason = compiled
        .evaluate(
            input.to_string(),
            "report".to_owned(),
            Some("sql".to_owned()),
        )
        .expect_err("the SQL engine refuses")
        .reason;
    assert!(
        reason.contains("Unsupported(three-valued violation.each)"),
        "{reason}"
    );
}

#[test]
fn each_enumerates_every_range_a_relation_binds() {
    let ast = json!({ "rule": {
        "kind": "call_expression",
        "has": { "field": "arguments", "has": { "kind": "identifier", "pattern": "$A" } },
    } });
    let source = "f(a, b, a);\ng();\n";
    assert_eq!(
        agreed("typescript", ast, Some("A"), source),
        ["f(a, b, a) A=a", "f(a, b, a) A=a", "f(a, b, a) A=b"]
    );
}

#[test]
fn each_stops_where_the_binding_repeats_or_goes_unbound() {
    // Bound at the root by its pattern: excluding it leaves the same binding, which ends it.
    let ast = json!({ "rule": { "pattern": "$A($$$)" } });
    assert_eq!(
        agreed("typescript", ast, Some("A"), "f(a);\n"),
        ["f(a) A=f"]
    );
    // An unbound first match is visited once; a later unbound one ends the enumeration.
    let ast = json!({ "rule": { "kind": "call_expression", "any": [
        { "has": { "field": "arguments", "has": { "kind": "identifier", "pattern": "$A" } } },
        { "kind": "call_expression" },
    ] } });
    assert_eq!(
        agreed("typescript", ast, Some("A"), "f(a, b);\ng();\n"),
        ["f(a, b) A=a", "f(a, b) A=b", "g()"]
    );
}

#[test]
fn each_follows_the_first_match_order_of_its_rounds() {
    // `matcher-semantics.md` §1F: a round binds the next candidate the first term accepts; where
    // a later term then rejects it, the round fails and the enumeration ends, though a later
    // candidate (`b`) would have passed.
    let ast = json!({ "rule": { "kind": "call_expression", "all": [
        { "has": { "field": "arguments", "has": { "kind": "identifier", "pattern": "$A" } } },
        { "has": { "field": "arguments", "has": { "kind": "identifier", "regex": "^[ab]$", "pattern": "$A" } } },
    ] } });
    assert_eq!(
        agreed("typescript", ast, Some("A"), "f(a, c, b);\n"),
        ["f(a, c, b) A=a"]
    );
    // The other captures are those bound with the candidate each round accepts.
    let ast = json!({ "rule": { "kind": "call_expression", "all": [
        { "has": { "field": "arguments", "has": { "kind": "identifier", "pattern": "$A" } } },
        { "has": { "field": "arguments", "has": { "kind": "number", "pattern": "$B" } } },
    ] } });
    assert_eq!(
        agreed("typescript", ast, Some("A"), "f(a, 1, b, 2);\n"),
        ["f(a, 1, b, 2) A=a B=1", "f(a, 1, b, 2) A=b B=1"]
    );
}

#[test]
fn each_excludes_nothing_inside_negations_counts_and_unpassed_calls() {
    // The `not` probe binds `$A` freely though `a` is excluded in the second round.
    let ast = json!({ "rule": { "kind": "call_expression", "all": [
        { "has": { "field": "arguments", "has": { "kind": "identifier", "pattern": "$A" } } },
        { "not": { "has": { "field": "arguments", "has": { "kind": "number" } } } },
        { "count": { "has": { "stopBy": "end", "kind": "identifier", "pattern": "$A" }, "min": 1 } },
    ] } });
    assert_eq!(
        agreed("typescript", ast, Some("A"), "f(a, b);\n"),
        ["f(a, b) A=a", "f(a, b) A=b"]
    );
    // A call the capture is passed to excludes it under its parameter; one it is not passed to
    // excludes nothing.
    let ast = json!({
        "rule": { "kind": "call_expression", "all": [
            { "matches": { "arg": { "P": "$A" } } },
            { "matches": { "arg": { "P": "$B" } } },
        ] },
        "utils": { "arg": { "params": ["P"], "rule": {
            "has": { "field": "arguments", "has": { "kind": "identifier", "pattern": "$P" } },
        } } },
    });
    assert_eq!(
        agreed("typescript", ast, Some("A"), "f(a, b);\n"),
        ["f(a, b) A=a B=a", "f(a, b) A=b B=a"]
    );
}

#[test]
fn each_excludes_inside_a_recursive_utility() {
    let ast = json!({
        "rule": { "kind": "call_expression", "has": { "field": "arguments", "matches": "leaf" } },
        "utils": { "leaf": { "any": [
            { "kind": "identifier", "pattern": "$A" },
            { "has": { "matches": "leaf" } },
        ] } },
    });
    let source = "f([a, [b]], c);\n";
    let found = agreed("typescript", ast, Some("A"), source);
    assert!(found.len() >= 2, "{found:?}");
}

#[test]
fn each_over_a_regex_group_reports_one_finding_per_group_match() {
    let ast = json!({ "rule": { "kind": "identifier", "regex": "(?P<PART>[a-z]+)_" } });
    let found = agreed("typescript", ast, Some("PART"), "const ab_cd_ef = 1;\n");
    assert_eq!(found.len(), 2, "{found:?}");
}

#[test]
fn the_capacity_limits_are_far_above_real_patterns_and_calls() {
    // A pattern with 40 captures and a call mapping six parameters to one capture.
    let names: Vec<String> = (0..40).map(|index| format!("$V{index}")).collect();
    let ast = json!({ "rule": { "pattern": format!("f({})", names.join(", ")) } });
    let arguments: Vec<String> = (0..40).map(|index| format!("a{index}")).collect();
    let source = format!("f({});\n", arguments.join(", "));
    assert_eq!(agreed("typescript", ast, None, &source).len(), 1);
    let params: Vec<String> = (0..6).map(|index| format!("P{index}")).collect();
    let pattern = format!(
        "$P0({})",
        params[1..]
            .iter()
            .map(|param| format!("${param}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mapping: serde_json::Map<String, Value> = params
        .iter()
        .map(|param| (param.clone(), json!("$X")))
        .collect();
    let ast = json!({
        "rule": { "matches": { "all": mapping } },
        "utils": { "all": { "params": params, "rule": { "pattern": pattern } } },
    });
    assert_eq!(
        agreed(
            "typescript",
            ast,
            None,
            "x(x, x, x, x, x);\nx(x, y, x, x, x);\n"
        ),
        ["x(x, x, x, x, x) X=x"]
    );
}

#[test]
fn a_recursion_level_runs_a_bounded_number_of_statements_per_batch() {
    use crate::code_rule_dsl::test_support::{compiled, grammar_matchers};
    // A capture-free recursion (a per-batch stage) and one that binds a capture (a frame loop).
    let rules = [
        json!({
            "rule": { "kind": "call_expression", "matches": "tail" },
            "utils": { "tail": { "any": [
                { "kind": "identifier", "regex": "^x$" },
                { "kind": "member_expression", "has": { "field": "object", "matches": "tail" } },
                { "kind": "call_expression", "has": { "field": "function", "matches": "tail" } },
            ] } },
        }),
        json!({
            "rule": { "kind": "call_expression", "has": { "field": "function", "matches": "base" } },
            "utils": { "base": { "any": [
                { "kind": "identifier", "pattern": "$A" },
                { "kind": "member_expression", "has": { "field": "object", "matches": "base" } },
            ] } },
        }),
    ];
    let mut passes = Vec::new();
    for rule in rules {
        let rule = compiled(rule);
        let grammars = vec![&grammar_matchers(&rule)[0]];
        let program = crate::code_rule_dsl::sql::compile(&grammars).expect("compiles");
        passes.extend(program.loop_statements());
    }
    eprintln!("statements per recursion level: {passes:?}");
    assert_eq!(passes, [61, 43]);
}
