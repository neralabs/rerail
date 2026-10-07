//! Port-shaped rules for ESLint's rules that read static values and property names, against
//! where ESLint 10.11.0 itself reports them on the fixtures in `fixtures/ecma/value/eslint`
//! (`fixtures/ecma/value/generate_eslint.mjs`).

use crate::policy_dsl::oracle;
use crate::policy_dsl::test_support::*;
use serde_json::{Value, json};

const ORACLE: &str = oracle!("ecma/value/eslint.expected.json");

/// Where a TypeScript query reports on a TypeScript or TSX fixture: each finding's start byte,
/// sorted.
fn starts(language: &str, ast: Value, anchor: &str, source: &str) -> Vec<u64> {
    let mut ast = ast;
    ast["language"] = json!("typescript");
    let document = check_document(json!({
        "id": "port",
        "forbid": { "ast": ast },
        "finding": { "anchor": anchor, "code": "PORT", "message": "port", "severity": "error" },
    }));
    let compiled = compile_document_with(
        document,
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let path = if language == "tsx" {
        "fixture.tsx"
    } else {
        "fixture.ts"
    };
    let result = evaluate_file(&compiled, language, path, source);
    assert_eq!(result["failures"], json!([]), "{result}");
    let mut starts = result["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|finding| finding["span"]["startByte"].as_u64().expect("start"))
        .collect::<Vec<_>>();
    starts.sort_unstable();
    starts.dedup();
    starts
}

fn expected(fixture: &str) -> Vec<u64> {
    let oracle: Value = serde_json::from_str(ORACLE).expect("oracle JSON");
    assert_eq!(oracle["versions"]["eslint"], "10.11.0");
    oracle[fixture]
        .as_array()
        .unwrap_or_else(|| panic!("{fixture} in the oracle"))
        .iter()
        .map(|start| start.as_u64().expect("start"))
        .collect()
}

/// no-self-assign with `props`: a member assigned to itself, as ESLint's `isSameReference` reads
/// it. Where the target has a static name, the source has the same name on the same object;
/// where it has none (`a[b]`), the two are the same syntax over names, literals and members. `key` equal to itself asks whether a
/// capture has a static name: a nameless key equals nothing.
#[test]
fn no_self_assign_members_by_static_name_or_syntax() {
    let named = |capture: &str| json!({ "compare": { "left": capture, "op": "eq", "right": capture, "text": "key" } });
    let ast = json!({
        "rule": {
            "kind": "assignment_expression",
            "pattern": "$L = $R",
            "any": [
                { "all": [
                    { "any": [
                        { "pattern": "$O.$X = $P.$Y" }, { "pattern": "$O.$X = $P[$Y]" },
                        { "pattern": "$O[$X] = $P.$Y" }, { "pattern": "$O[$X] = $P[$Y]" },
                    ] },
                    named("L"),
                    { "compare": { "left": "L", "op": "eq", "right": "R", "text": "key" } },
                    { "compare": { "left": "O", "op": "eq", "right": "P", "text": "tokens" } },
                ] },
                { "all": [
                    { "not": named("L") },
                    { "pattern": "$O[$K] = $P[$J]" },
                    { "compare": { "left": "L", "op": "eq", "right": "R", "text": "tokens" } },
                ] },
            ],
        },
        "constraints": {
            "L": { "any": [{ "kind": "member_expression" }, { "kind": "subscript_expression" }] },
            // isSameReference compares names, literals, `this` and members; nothing else is the same.
            "K": { "any": [
                { "kind": "identifier" }, { "kind": "this" }, { "kind": "member_expression" },
                { "kind": "subscript_expression" }, { "kind": "string" }, { "kind": "number" },
            ] },
        },
    });
    let fixture = "no-self-assign.ts";
    let source = oracle!("ecma/value/eslint/no-self-assign.ts");
    assert_eq!(
        starts("typescript", ast, "capture.R", source),
        expected(fixture)
    );
}

/// no-implied-eval: a global `setTimeout`, `setInterval` or `execScript` called with a first
/// argument (`node.arguments[0]`, a spread included: `mode: "estree"`) that getStaticValue folds to
/// a string, calls of builtins included (`String('x=1')`, `'a'.toUpperCase()`), or that ESLint's
/// `isEvaluatedString` reads as one. A destructured name has no static value.
#[test]
fn no_implied_eval_folds_builtin_calls() {
    let globals =
        json!({ "setTimeout": "readonly", "setInterval": "readonly", "execScript": "readonly" });
    let ast = json!({
        "languageOptions": { "globals": globals },
        "utils": {
            "evaluated": { "any": [
                { "kind": "string" },
                { "kind": "template_string" },
                { "kind": "parenthesized_expression", "has": { "stopBy": "neighbor", "matches": "evaluated" } },
                { "kind": "binary_expression", "all": [
                    { "has": { "stopBy": "neighbor", "regex": "^\\+$" } },
                    { "has": { "stopBy": "neighbor", "matches": "evaluated" } },
                ] },
            ] },
        },
        "rule": {
            "kind": "call_expression",
            "pattern": "$F($$$)",
            "arguments": { "signature": ["code"], "bind": { "S": "code" }, "mode": "estree" },
        },
        "constraints": {
            "F": { "kind": "identifier", "resolves": { "global": ["setTimeout", "setInterval", "execScript"] } },
            "S": { "any": [{ "value": { "type": "string", "follow": true } }, { "matches": "evaluated" }] },
        },
    });
    let fixture = "no-implied-eval.ts";
    let source = oracle!("ecma/value/eslint/no-implied-eval.ts");
    assert_eq!(
        starts("typescript", ast, "witness.span", source),
        expected(fixture)
    );
}

/// operator-assignment (`always`): `x = x op y`, or `x = y op x` for a commutative operator,
/// where the target is the same reference as the operand by ESLint's `isSameReference` without
/// static computed keys (`reference-no-static-keys`): `x.y = x['y'] - 1` is not reported. The
/// assignment is the binary expression's nearest ancestor past parentheses; the `all` that
/// compares reads `$L`, which the `inside` binds, so it runs after it.
#[test]
fn operator_assignment_compares_references() {
    let binary = |operators: &[&str]| json!({ "any": operators.iter().map(|operator| json!({ "pattern": format!("$A {operator} $B") })).collect::<Vec<_>>() });
    let same = |side: &str| json!({ "compare": { "left": "L", "op": "eq", "right": side, "text": "reference-no-static-keys" } });
    let commutative = ["*", "&", "^", "|"];
    let ast = json!({
        "rule": {
            "kind": "binary_expression",
            "inside": {
                "stopBy": { "not": { "kind": "parenthesized_expression" } },
                "kind": "assignment_expression",
                "all": [{ "pattern": "$L = $R" }, { "pattern": "$W" }],
            },
            "all": [
                binary(&["+", "-", "/", "%", "<<", ">>", ">>>", "**", "*", "&", "^", "|"]),
                { "any": [same("A"), { "all": [binary(&commutative), same("B")] }] },
            ],
        },
    });
    let fixture = "operator-assignment.ts";
    let source = oracle!("ecma/value/eslint/operator-assignment.ts");
    assert_eq!(
        starts("typescript", ast, "capture.W", source),
        expected(fixture)
    );
}

/// no-useless-rename: an import, export or destructuring property renamed to its own name, names
/// compared as ESTree has them (`name`): escapes decoded, a string name by its value, a number key
/// never a name.
#[test]
fn no_useless_rename_compares_estree_names() {
    let same = json!({ "compare": { "left": "A", "op": "eq", "right": "B", "text": "name" } });
    let ast = json!({
        "rule": {
            "any": [
                { "pattern": "import { $A as $B } from 'm'" },
                { "pattern": "export { $A as $B }" },
                { "pattern": { "context": "const { $A: $B } = o", "selector": "pair_pattern" },
                  "not": { "has": { "stopBy": "neighbor", "kind": "assignment_pattern" } } },
                { "kind": "pair_pattern", "all": [
                    { "has": { "stopBy": "neighbor", "pattern": "$A",
                               "any": [{ "kind": "property_identifier" }, { "kind": "string" }, { "kind": "number" }] } },
                    { "has": { "stopBy": "neighbor", "kind": "assignment_pattern",
                               "has": { "stopBy": "neighbor", "kind": "identifier", "pattern": "$B" } } },
                ] },
            ],
            "all": [same],
        },
        "constraints": { "A": { "not": { "kind": "computed_property_name" } } },
    });
    let fixture = "no-useless-rename.ts";
    let source = oracle!("ecma/value/eslint/no-useless-rename.ts");
    assert_eq!(
        starts("typescript", ast, "capture.A", source),
        expected(fixture)
    );
}

/// no-self-assign's identifiers and array destructuring (M12b): the i-th element of an array
/// pattern paired with the i-th element of the array (`compare` `text: "index"`, holes counted,
/// parentheses transparent), up to a spread on the right; a trailing spread pairs with the rest.
/// Each finding is the right element, as ESLint reports it.
#[test]
fn no_self_assign_pairs_array_elements_by_index() {
    let unparen = json!({ "not": { "kind": "parenthesized_expression" } });
    let tokens = |left: &str, right: &str| json!({ "compare": { "left": left, "op": "eq", "right": right, "text": "tokens" } });
    let index = |left: &str, op: &str, right: &str| json!({ "compare": { "left": left, "op": op, "right": right, "text": "index" } });
    let assigned = |left: Value| json!({ "field": "right", "stopBy": unparen, "matches": "assign", "has": left });
    let ast = json!({
        "utils": {
            "assign": { "any": [
                { "kind": "assignment_expression" },
                { "kind": "augmented_assignment_expression", "has": { "regex": "^(&&=|\\|\\|=|\\?\\?=)$" } },
            ] },
        },
        "rule": {
            "kind": "identifier",
            "pattern": "$R",
            "any": [
                { "inside": assigned(json!({
                    "field": "left", "stopBy": unparen, "kind": "identifier", "pattern": "$L", "all": [tokens("L", "R")],
                })) },
                { "inside": {
                    "stopBy": unparen,
                    "kind": "array",
                    "not": { "has": { "kind": "spread_element", "pattern": "$S", "all": [index("S", "lt", "R")] } },
                    "inside": assigned(json!({ "field": "left", "kind": "array_pattern", "has": {
                        "kind": "identifier", "pattern": "$L", "all": [tokens("L", "R"), index("L", "eq", "R")],
                    } })),
                } },
                { "inside": {
                    "kind": "spread_element",
                    "pattern": "$S",
                    "inside": {
                        "kind": "array",
                        "not": { "has": { "pattern": "$E", "all": [index("E", "gt", "S")] } },
                        "inside": assigned(json!({ "field": "left", "kind": "array_pattern", "has": {
                            "kind": "rest_pattern", "pattern": "$T", "all": [index("T", "eq", "S")],
                            "has": { "kind": "identifier", "pattern": "$L", "all": [tokens("L", "R")] },
                        } })),
                    },
                } },
            ],
        },
    });
    let fixture = "no-self-assign@arrays.ts";
    let source = oracle!("ecma/value/eslint/no-self-assign@arrays.ts");
    assert_eq!(
        starts("typescript", ast, "witness.span", source),
        expected(fixture)
    );
}

/// no-duplicate-imports with default options (M12b): an import whose module, trimmed as ESLint's
/// `getModule` trims it (a `transform`), an earlier import names too, where ESLint's merge rules
/// allow merging the two; the message interpolates the trimmed module. The shape is the ESLint
/// suite's port with the module compared by trimmed text instead of by cooked value.
#[test]
fn no_duplicate_imports_compares_trimmed_modules() {
    let trim = |source: &str| json!({ "replace": { "source": source, "replace": "^[\\s\\x{FEFF}]+|[\\s\\x{FEFF}]+$", "by": "" } });
    let prior = |except: Value| json!({ "follows": { "stopBy": "end", "all": [{ "matches": "import" }, except, { "matches": "same-module" }] } });
    let not_any = |ids: &[&str]| json!({ "not": { "any": ids.iter().map(|id| json!({ "matches": id })).collect::<Vec<_>>() } });
    let typed_default_or_named = |other: &str| json!({ "not": { "all": [{ "matches": "type-kind" }, { "matches": other }] } });
    let categories = |typed: bool| {
        let named_prior = if typed {
            json!({ "all": [not_any(&["cat-ns"]), typed_default_or_named("cat-default")] })
        } else {
            not_any(&["cat-ns"])
        };
        let default_prior = if typed {
            typed_default_or_named("cat-named")
        } else {
            json!({ "kind": "import_statement" })
        };
        json!({ "any": [
            { "all": [{ "matches": "cat-side" }, prior(json!({ "kind": "import_statement" }))] },
            { "all": [{ "matches": "cat-ns" }, prior(not_any(&["cat-named"]))] },
            { "all": [{ "matches": "cat-named" }, prior(named_prior)] },
            { "all": [{ "matches": "cat-default" }, prior(default_prior)] },
        ] })
    };
    let ast = json!({
        "utils": {
            "import": { "kind": "import_statement" },
            "type-kind": { "has": { "regex": "^type$" } },
            "cat-ns": { "has": { "kind": "import_clause", "has": { "kind": "namespace_import" } } },
            "cat-named": { "has": { "kind": "import_clause", "all": [
                { "has": { "kind": "named_imports" } }, { "not": { "has": { "kind": "namespace_import" } } },
            ] } },
            "cat-default": { "has": { "kind": "import_clause",
                "not": { "has": { "any": [{ "kind": "named_imports" }, { "kind": "namespace_import" }] } } } },
            "cat-side": { "not": { "has": { "kind": "import_clause" } } },
            "same-module": { "has": { "kind": "string", "has": { "kind": "string_fragment", "pattern": "$P" },
                "compare": { "left": "PT", "op": "eq", "right": "MT" } } },
        },
        "rule": {
            "all": [
                { "matches": "import" },
                { "has": { "kind": "string", "has": { "kind": "string_fragment", "pattern": "$M" } } },
                { "not": { "compare": { "left": "MT", "op": "eq", "right": "EMPTY" } } },
                { "any": [
                    { "all": [{ "matches": "type-kind" }, categories(true)] },
                    { "all": [{ "not": { "matches": "type-kind" } }, categories(false)] },
                ] },
            ],
        },
        "transform": {
            "MT": trim("$M"),
            "PT": trim("$P"),
            "EMPTY": { "substring": { "source": "$M", "endChar": 0 } },
        },
    });
    let fixture = "no-duplicate-imports.ts";
    let source = oracle!("ecma/value/eslint/no-duplicate-imports.ts");
    assert_eq!(
        starts("typescript", ast.clone(), "witness.span", source),
        expected(fixture)
    );
    let mut ast = ast;
    ast["language"] = json!("typescript");
    let document = check_document(json!({
        "id": "port",
        "forbid": { "ast": ast },
        "finding": { "anchor": "witness.span", "code": "PORT", "message": "'$MT' import is duplicated.", "severity": "error" },
    }));
    let compiled = compile_document_with(
        document,
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "typescript", "fixture.ts", source);
    let messages = result["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|finding| finding["message"].as_str().expect("message").to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        messages,
        ["m", "m", "n", "o", "p", "q", "s"]
            .map(|module| format!("'{module}' import is duplicated."))
    );
}

/// no-dupe-else-if (M12b): the ESLint suite's port, with its token comparisons made commutative
/// over `||` and `&&` (`commute`, ESLint's `equal`). A test is reported where an earlier test of
/// the chain equals it, has an `||` operand (at any depth) that equals it, or equals one of its
/// top-level `&&` operands. ESLint's cover of one test's `||` operands by several earlier tests
/// together is not this shape's (`a || b || c` then `c || a || b`), so the fixture leaves it out.
#[test]
fn no_dupe_else_if_compares_conditions_commutatively() {
    let equal = |left: &str| {
        json!({ "pattern": "$X", "compare": {
            "left": "X", "op": "eq", "right": left, "text": "tokens", "commute": ["||", "&&"],
        } })
    };
    let ast = json!({
        "utils": {
            "covers": { "any": [
                equal("C"),
                { "all": [
                    { "kind": "binary_expression" },
                    { "has": { "regex": "^\\|\\|$" } },
                    { "has": { "not": { "regex": "^\\|\\|$" }, "matches": "covers" } },
                ] },
                { "all": [{ "kind": "parenthesized_expression" }, { "has": { "matches": "covers" } }] },
                equal("L"),
                equal("R"),
            ] },
            "earlier": { "inside": { "kind": "else_clause", "inside": { "kind": "if_statement", "any": [
                { "has": { "kind": "parenthesized_expression", "has": { "matches": "covers" } } },
                { "matches": "earlier" },
            ] } } },
        },
        "rule": {
            "all": [
                { "not": { "kind": "parenthesized_expression" } },
                { "pattern": "$C" },
                { "any": [
                    { "all": [
                        { "kind": "binary_expression" },
                        { "has": { "regex": "^&&$" } },
                        { "has": { "all": [{ "not": { "regex": "^&&$" } }, { "pattern": "$L" }, { "precedes": { "regex": "^&&$" } }] } },
                        { "has": { "all": [{ "not": { "regex": "^&&$" } }, { "pattern": "$R" }, { "follows": { "regex": "^&&$" } }] } },
                    ] },
                    { "all": [{ "pattern": "$L" }, { "pattern": "$R" }] },
                ] },
                { "inside": { "kind": "if_statement", "stopBy": { "not": { "kind": "parenthesized_expression" } }, "matches": "earlier" } },
            ],
        },
    });
    let fixture = "no-dupe-else-if.ts";
    let source = oracle!("ecma/value/eslint/no-dupe-else-if.ts");
    // Before M12b the suite's port compared tokens in order and missed the commuted tests.
    let ordered: Value =
        serde_json::from_str(&ast.to_string().replace(r#""commute":["||","&&"],"#, ""))
            .expect("rule JSON");
    assert_eq!(
        starts("typescript", ordered, "witness.span", source),
        [19, 181, 206, 241, 482]
    );
    assert_eq!(
        starts("typescript", ast, "witness.span", source),
        expected(fixture)
    );
}

/// no-script-url: a string whose static value starts with `javascript:`, case aside. A JSX
/// attribute string is raw, with its character references decoded; a template counts without
/// substitutions and untagged.
#[test]
fn no_script_url_reads_jsx_attribute_strings_as_jsx_does() {
    let ast = json!({
        "rule": {
            "any": [
                { "kind": "string" },
                { "kind": "template_string", "all": [
                    { "not": { "has": { "kind": "template_substitution" } } },
                    { "not": { "inside": { "kind": "call_expression" } } },
                ] },
            ],
            "value": { "regex": "^(?i)javascript:" },
        },
    });
    let fixture = "no-script-url.tsx";
    let source = oracle!("ecma/value/eslint/no-script-url.tsx");
    assert_eq!(
        starts("tsx", ast, "witness.span", source),
        expected(fixture)
    );
}

/// no-dupe-keys, read through key nodes: a key of an object literal (a plain key, a method's
/// name, a shorthand, or a computed key's expression without brackets and parentheses, which is
/// where ESLint reports) named as a key of an earlier property of the same object. A key without
/// a static name is never a duplicate.
#[test]
fn no_dupe_keys_through_key_nodes() {
    let same = json!({ "compare": { "left": "J", "op": "eq", "right": "K", "text": "key" } });
    let ast = json!({
        "utils": {
            "key": { "any": [
                { "kind": "shorthand_property_identifier", "inside": { "kind": "object" } },
                { "any": [{ "kind": "property_identifier" }, { "kind": "string" }, { "kind": "number" }],
                  "inside": { "kind": "pair", "inside": { "kind": "object" } },
                  "precedes": { "regex": "^:$" } },
                { "kind": "property_identifier", "inside": { "kind": "method_definition", "inside": { "kind": "object" } } },
                { "not": { "kind": "parenthesized_expression" },
                  "inside": {
                      "stopBy": { "not": { "kind": "parenthesized_expression" } },
                      "kind": "computed_property_name",
                      "inside": { "any": [{ "kind": "pair" }, { "kind": "method_definition" }], "inside": { "kind": "object" } },
                  } },
            ] },
            "prior": { "any": [
                { "kind": "shorthand_property_identifier", "pattern": "$J", "all": [same.clone()] },
                { "all": [
                    { "any": [{ "kind": "pair" }, { "kind": "method_definition" }] },
                    { "any": [
                        { "has": { "matches": "key", "pattern": "$J", "all": [same.clone()] } },
                        { "has": { "kind": "computed_property_name", "has": {
                            "stopBy": { "not": { "kind": "parenthesized_expression" } },
                            "matches": "key", "pattern": "$J", "all": [same],
                        } } },
                    ] },
                ] },
            ] },
        },
        "rule": {
            "matches": "key",
            "pattern": "$K",
            "any": [
                { "kind": "shorthand_property_identifier",
                  "count": { "follows": { "stopBy": "end", "matches": "prior" }, "min": 1 } },
                { "inside": {
                    "stopBy": { "kind": "object" },
                    "any": [{ "kind": "pair" }, { "kind": "method_definition" }],
                    "count": { "follows": { "stopBy": "end", "matches": "prior" }, "min": 1 },
                } },
            ],
        },
    });
    let fixture = "no-dupe-keys.ts";
    let source = oracle!("ecma/value/eslint/no-dupe-keys.ts");
    assert_eq!(
        starts("typescript", ast, "witness.span", source),
        expected(fixture)
    );
}
