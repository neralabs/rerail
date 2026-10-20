//! `embedded` and `host` between a file's own tree and the program it embeds, through utilities
//! as written inline.

use crate::code_rule_dsl::test_support::*;
use serde_json::{Value, json};

const REFS: &str = "<template><div ref=\"a\"></div><p ref=\"b\"></p></template>\n<script setup>\nconst a = 1\n</script>\n";

fn compiled(ast: Value) -> crate::code_rule_dsl::CompiledCodeRuleDsl {
    compile_document_with(
        clause_document(json!({ "id": "c", "forbid": { "ast": ast }, "violation": violation() })),
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason))
}

fn reported(ast: Value, source: &str) -> Vec<String> {
    let result = evaluate_file(&compiled(ast), "vue", "C.vue", source);
    assert_eq!(result["failures"], json!([]), "{result}");
    matched_texts(&result, source)
}

/// A `ref` value whose name the script declares: the template's value is captured on the host,
/// the declaration found in the program, and the two compared by `host` in the host's
/// environment, with the program's capture exported into it.
fn declared_ref(program_rule: Value) -> Value {
    json!({
        "kind": "quoted_attribute_value",
        "pattern": "$V",
        "inside": { "kind": "attribute", "inside": { "stopBy": "end", "kind": "document",
            "embedded": program_rule } }
    })
}

#[test]
fn a_host_rule_reads_the_host_environment_inline_and_through_a_utility() {
    let program = json!({ "all": [
        { "kind": "identifier", "inside": { "kind": "variable_declarator" } },
        { "pattern": "$N" },
        { "host": { "compare": { "left": "V", "op": "eq", "right": "N", "text": "unquoted" } } }
    ] });
    let inline = json!({ "language": "vue", "rule": declared_ref(program.clone()) });
    assert_eq!(reported(inline, REFS), ["\"a\" N=a V=\"a\""]);
    let through = json!({
        "language": "vue",
        "rule": declared_ref(json!({ "matches": "declares" })),
        "utils": { "declares": program },
    });
    assert_eq!(reported(through, REFS), ["\"a\" N=a V=\"a\""]);
}

/// A utility reached through `embedded` binds in the program's environment: its own captures
/// are visible to its comparisons, under a `field` relation too.
#[test]
fn a_utility_reached_through_embedded_binds_its_own_names() {
    let source = "<template><p v-if=\"ok\"></p></template>\n<script setup>\nif (ok) { const f = (x) => x }\n</script>\n";
    let ast = json!({
        "language": "vue",
        "rule": { "kind": "directive_attribute", "embedded": { "matches": "aliased" } },
        "utils": { "aliased": { "all": [
            { "kind": "identifier" },
            { "pattern": "$REF" },
            { "inside": { "stopBy": "end", "kind": "if_statement", "has": {
                "stopBy": "neighbor", "field": "consequence", "kind": "statement_block",
                "has": { "stopBy": "end", "kind": "arrow_function", "has": { "stopBy": "end", "all": [
                    { "kind": "identifier" },
                    { "pattern": "$AL" },
                    { "compare": { "left": "REF", "op": "eq", "right": "AL" } }
                ] } } } } }
        ] } },
    });
    assert!(reported(ast, source).is_empty());
}

/// A utility no rule reaches may be written for either document of an embedding language.
#[test]
fn an_unreached_utility_may_be_the_programs() {
    let ast = json!({
        "language": "vue",
        "rule": { "kind": "quoted_attribute_value", "regex": "^\"b\"$" },
        "utils": { "program-only": { "kind": "lexical_declaration" } },
    });
    assert_eq!(reported(ast, REFS), ["\"b\""]);
}

/// A define's utilities of an embedding language are whole in either document's grammar.
#[test]
fn a_define_holds_utilities_of_the_program() {
    let document = json!({
        "format": "rerail.policy/v1", "id": "d", "revision": 1, "mode": "block",
        "clauses": [
            { "id": "shared", "define": { "language": "vue", "utils": {
                "declarator": { "kind": "variable_declarator" }
            } } },
            { "id": "c", "forbid": { "ast": { "language": "vue", "embedded": "script-block",
                "rule": { "kind": "identifier", "inside": { "matches": "declarator" } } } },
              "violation": violation() }
        ]
    });
    let compiled = compile_document_with(
        document,
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "vue", "C.vue", REFS);
    assert_eq!(matched_texts(&result, REFS), ["a"]);
}

/// A transform's line numbers in a rule over an embedded program are the file's: the program
/// starts with the script, but its first line is the component's fourth.
#[test]
fn a_programs_line_numbers_are_the_files() {
    let source =
        "<template>\n  <p>{{ a }}</p>\n</template>\n<script setup>\nconst a = 1\n</script>\n";
    let ast = json!({
        "language": "vue",
        "embedded": "script-block",
        "rule": { "kind": "variable_declarator", "pattern": "$D" },
        "transform": { "L": { "line": { "source": "$D" } } },
    });
    let document = clause_document(json!({
        "id": "c",
        "forbid": { "ast": ast },
        "violation": { "anchor": "witness.span", "code": "X", "message": "line $L" },
    }));
    let compiled = compile_document_with(
        document,
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "vue", "C.vue", source);
    let violations = result["violations"].as_array().expect("violations");
    assert_eq!(violations.len(), 1, "{result}");
    assert_eq!(violations[0]["message"], "line 5", "{result}");
    assert_eq!(violations[0]["span"]["startLine"], 4, "{result}");
}

/// A rule over the program is whole in some dialect the program may be in, as a query is in one
/// of its own: a misspelt kind under `embedded` is refused, not read as matching nothing (which a
/// `not` would read as matching everything).
#[test]
fn a_gap_under_embedded_is_the_querys() {
    let document = clause_document(json!({
        "id": "c",
        "forbid": { "ast": { "language": "blade", "rule": { "kind": "echo", "embedded": {
            "kind": "function_call_expression",
            "not": { "kind": "functon_call_expression" }
        } } } },
        "violation": violation(),
    }));
    let reason = rejection(compile_document_with(
        document,
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    ));
    assert!(reason.contains("functon_call_expression"), "{reason}");
}

/// `violation.each` enumerates a capture `embedded` exports: one violation per program binding,
/// each at the file bytes it stands for.
#[test]
fn each_enumerates_what_embedded_exports() {
    let source = "<template>\n  <p>{{ a + b }}</p>\n</template>\n";
    let document = clause_document(json!({
        "id": "c",
        "forbid": { "ast": { "language": "vue", "rule": {
            "kind": "interpolation",
            "embedded": { "kind": "identifier", "pattern": "$N" }
        } } },
        "violation": { "anchor": "capture.N", "each": "N", "code": "X", "message": "$N" },
    }));
    let compiled = compile_document_with(
        document,
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "vue", "C.vue", source);
    assert_eq!(result["failures"], json!([]), "{result}");
    let messages = result["violations"]
        .as_array()
        .expect("violations")
        .iter()
        .map(|violation| violation["message"].as_str().expect("message").to_owned())
        .collect::<Vec<_>>();
    assert_eq!(messages, ["a", "b"], "{result}");
}

/// A define's utility may read a literal only the program's grammar hosts (`parse` of an ESLint
/// directive comment): a Vue component's own tree hosts none, but the utility is whole in the
/// program's.
#[test]
fn a_define_holds_a_programs_parse_utility() {
    let source = "<script setup>\n/* eslint-disable no-console */\nconst a = 1\n</script>\n";
    let document = json!({
        "format": "rerail.policy/v1", "id": "d", "revision": 1, "mode": "block",
        "clauses": [
            { "id": "shared", "define": { "language": "vue", "utils": {
                "directive": { "kind": "comment", "parse": { "grammar": "eslintDirective", "kind": "directive" } }
            } } },
            { "id": "c", "forbid": { "ast": { "language": "vue", "embedded": "script-block",
                "rule": { "matches": "directive" } } },
              "violation": violation() }
        ]
    });
    let compiled = compile_document_with(
        document,
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "vue", "C.vue", source);
    assert_eq!(
        matched_texts(&result, source),
        ["/* eslint-disable no-console */"]
    );
}

/// Zero-width positions are owned half-open: a point at the boundary of two siblings is the one
/// starting there, never the one ending there; the file's end is the last node's.
#[test]
fn a_zero_width_point_belongs_to_the_node_starting_there() {
    use ast_grep_language::LanguageExt;
    let source = "a(b);c(d);";
    let root = crate::code_rule_dsl::language::GrammarDialect::TypeScript.ast_grep(source);
    let text = |at: usize| super::innermost(root.root(), &(at..at)).text().into_owned();
    // `a(b);` ends where `c(d);` starts.
    assert_eq!(text(5), "c");
    assert_eq!(text(0), "a");
    // The file's end belongs to the last statement's last token.
    assert_eq!(text(source.len()), ";");
    // A range of width is held by the innermost node holding it whole.
    assert_eq!(super::innermost(root.root(), &(2..3)).text(), "b");
}

/// A program node standing for file ranges apart (a component's script and the template after it)
/// reports the range it starts in, never a span widened over the closing tag between them.
#[test]
fn a_node_standing_for_ranges_apart_reports_where_it_starts() {
    let source = "<script>\nlet a = 1\n</script>\n<template><p>{{ a }}</p></template>\n";
    let document = clause_document(json!({
        "id": "c",
        "forbid": { "ast": { "language": "vue", "embedded": "script-block",
            "rule": { "kind": "program" } } },
        "violation": violation(),
    }));
    let compiled = compile_document_with(
        document,
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "vue", "C.vue", source);
    assert_eq!(
        matched_texts(&result, source),
        ["<script>\nlet a = 1\n"],
        "{result}"
    );
}
