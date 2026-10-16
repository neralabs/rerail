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
