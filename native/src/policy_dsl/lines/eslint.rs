//! Port-shaped rules for ESLint's rules that read lines (M12a's `layout`) and directive comments
//! (`parse` with `eslintDirective`), against where ESLint 10.11.0 itself reports them on the
//! fixtures in `rerail-fixtures/ecma/lines/eslint` (`rerail-fixtures/ecma/lines/generate_eslint.mjs`).

use crate::policy_dsl::oracle;
use crate::policy_dsl::test_support::*;
use serde_json::{Value, json};

const ORACLE: &str = oracle!("ecma/lines/eslint.expected.json");

/// Where a TypeScript query reports on a fixture: each finding's start byte, sorted.
fn starts(ast: Value, source: &str) -> Vec<u64> {
    starts_at(ast, "witness.span", source)
}

/// `starts`, with the finding anchored at `anchor`.
fn starts_at(ast: Value, anchor: &str, source: &str) -> Vec<u64> {
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
    let result = evaluate_file(&compiled, "typescript", "fixture.ts", source);
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

/// A token, comments included, on one side of the comment and on its line.
fn same_line(side: &str) -> Value {
    json!({ "layout": { side: { "to": "token", "comments": true, "breaks": { "eq": 0 } } } })
}

/// no-inline-comments: a comment with a token or comment before it on its first line, or after it
/// on its last (ESLint's preamble and postamble).
#[test]
fn no_inline_comments_reads_the_comment_s_lines() {
    let ast =
        json!({ "rule": { "kind": "comment", "any": [same_line("before"), same_line("after")] } });
    assert_eq!(
        starts(ast, oracle!("ecma/lines/eslint/no-inline-comments.ts")),
        expected("no-inline-comments.ts")
    );
}

/// capitalized-comments with `ignoreInlineComments`: a lowercase comment, unless tokens (comments
/// included) sit on both sides of it on its lines (`isInlineComment`).
#[test]
fn capitalized_comments_exempts_inline_comments_by_their_lines() {
    let ast = json!({ "rule": {
        "kind": "comment",
        "regex": "^(//|/\\*)\\s*\\p{Ll}",
        "not": { "all": [same_line("before"), same_line("after")] },
    } });
    assert_eq!(
        starts(
            ast,
            oracle!("ecma/lines/eslint/capitalized-comments@inline.ts")
        ),
        expected("capitalized-comments@inline.ts")
    );
}

/// sort-keys with `allowLineSeparatedGroups`: a key below the previous property's, unless a blank
/// line (no token or comment on it) separates the two properties. A line inside a comment is not
/// blank; CRLF ends a line once.
#[test]
fn sort_keys_restarts_after_a_blank_line() {
    let key = |name: &str| json!({ "field": "key", "pattern": format!("${name}") });
    let ast = json!({ "rule": {
        "kind": "pair",
        "has": key("K"),
        "follows": { "stopBy": "end", "kind": "pair", "has": key("P") },
        "compare": { "left": "P", "op": "gt", "right": "K" },
        "not": { "layout": { "before": { "to": "sibling", "blank": { "min": 1 } } } },
    } });
    let source = oracle!("ecma/lines/eslint/sort-keys@groups.ts");
    assert!(source.contains("\r\n"), "the fixture keeps its CRLF block");
    assert_eq!(starts(ast, source), expected("sort-keys@groups.ts"));
}

/// no-unexpected-multiline for calls and computed members: the `(` or `[` on a line after the
/// token before it (comments passed over, as `getTokenBefore` does), unless optional (a call's
/// `?.` is a token, a member's an `optional_chain`) or without arguments.
#[test]
fn no_unexpected_multiline_reads_the_line_before_the_bracket() {
    let broken = json!({ "layout": { "before": { "to": "token", "breaks": { "min": 1 } } } });
    let not_optional = json!({ "not": { "has": { "stopBy": "neighbor",
        "any": [{ "kind": "optional_chain" }, { "regex": "^\\?\\.$" }] } } });
    let mut call = json!({ "regex": "^\\($", "inside": { "kind": "arguments", "stopBy": "neighbor",
        "has": { "stopBy": "neighbor", "not": { "regex": "^[(),]$" } },
        "inside": { "kind": "call_expression", "stopBy": "neighbor", "all": [not_optional.clone()] } } });
    call["layout"] = broken["layout"].clone();
    let mut member = json!({ "regex": "^\\[$", "inside": { "kind": "subscript_expression", "stopBy": "neighbor",
        "all": [not_optional] } });
    member["layout"] = broken["layout"].clone();
    let ast = json!({ "rule": { "any": [call, member] } });
    assert_eq!(
        starts(ast, oracle!("ecma/lines/eslint/no-unexpected-multiline.ts")),
        expected("no-unexpected-multiline.ts")
    );
}

/// no-useless-concat: `+` between two string literals on one line: the left operand's end and the
/// right one's start (read from the operator, which sits between them).
#[test]
fn no_useless_concat_reads_the_operands_lines() {
    let literal = |field: &str, name: &str| json!({ "has": { "field": field, "any": [{ "kind": "string" }, { "kind": "template_string" }], "pattern": format!("${name}") } });
    let ast = json!({ "rule": {
        "regex": "^\\+$",
        "inside": { "kind": "binary_expression", "stopBy": "neighbor", "all": [literal("left", "L"), literal("right", "R")] },
        "layout": {
            "before": { "to": "capture.L", "breaks": { "eq": 0 } },
            "after": { "to": "capture.R", "breaks": { "eq": 0 } },
        },
    } });
    assert_eq!(
        starts(ast, oracle!("ecma/lines/eslint/no-useless-concat.ts")),
        expected("no-useless-concat.ts")
    );
}

/// no-redeclare with `builtinGlobals` in a script: a `/* global */` comment naming a variable a
/// `var` declares or a builtin, reported at the name inside the comment (ESLint's
/// `getNameLocationInGlobalDirectiveComment`). Before M12a a finding stood on a node and a comment
/// had none. The comment's first name only: a later name's check reads the host, outside the parsed
/// tree, so `each` cannot reach it.
#[test]
fn no_redeclare_anchors_inside_a_global_comment() {
    let names = |rule: Value| json!({ "grammar": "eslintDirective", "value": { "regex": "^globals?$" }, "has": rule });
    let ast = json!({ "rule": {
        "kind": "comment",
        "parse": names(json!({ "kind": "item", "capture": "N" })),
        "any": [
            { "inside": { "kind": "program", "has": { "stopBy": "end", "kind": "variable_declarator",
                "has": { "field": "name", "pattern": "$V" },
                "compare": { "left": "V", "op": "eq", "right": "N" } } } },
            { "parse": names(json!({ "kind": "item", "regex": "^(Array|Object|String)$" })) },
        ],
    } });
    assert_eq!(
        starts_at(
            ast,
            "capture.N",
            oracle!("ecma/lines/eslint/no-redeclare@globals.ts")
        ),
        expected("no-redeclare@globals.ts")
    );
}

/// Where a TypeScript query reports on a fixture and with which message, anchored at the witness's
/// start, in document order.
fn messages(ast: Value, message: &str, source: &str) -> Vec<(u64, String)> {
    let mut ast = ast;
    ast["language"] = json!("typescript");
    let document = check_document(json!({
        "id": "port",
        "forbid": { "ast": ast },
        "finding": { "anchor": "witness.start", "code": "PORT", "message": message, "severity": "error" },
    }));
    let compiled = compile_document_with(
        document,
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "typescript", "fixture.ts", source);
    assert_eq!(result["failures"], json!([]), "{result}");
    let mut found = result["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|finding| {
            (
                finding["span"]["startByte"].as_u64().expect("start"),
                finding["message"].as_str().expect("message").to_owned(),
            )
        })
        .collect::<Vec<_>>();
    found.sort();
    found
}

fn expected_messages(fixture: &str) -> Vec<(u64, String)> {
    let oracle: Value = serde_json::from_str(ORACLE).expect("oracle JSON");
    oracle["messages"][fixture]
        .as_array()
        .unwrap_or_else(|| panic!("{fixture}'s messages in the oracle"))
        .iter()
        .map(|entry| {
            (
                entry[0].as_u64().expect("start"),
                entry[1].as_str().expect("message").to_owned(),
            )
        })
        .collect()
}

/// max-params and max-statements (`max: 1`) count the parameters and statements, not the comments
/// between them, which a `$$$` capture holds (the fix round after the suites scored M12).
#[test]
fn counts_in_messages_pass_over_comments() {
    let params = json!({
        "rule": { "kind": "function_declaration", "has": { "field": "name", "pattern": "$N" },
            "all": [{ "has": { "field": "parameters",
                "pattern": { "context": "function f($$$P) {}", "selector": "formal_parameters" },
                "count": { "has": { "stopBy": "neighbor", "any": [{ "kind": "required_parameter" }, { "kind": "optional_parameter" }] }, "min": 2 } } }] },
        "transform": { "C": { "count": { "source": "$$$P" } } },
    });
    assert_eq!(
        messages(
            params,
            "Function '$N' has too many parameters ($C). Maximum allowed is 1.",
            oracle!("ecma/lines/eslint/max-params@comments.ts")
        ),
        expected_messages("max-params@comments.ts")
    );
    let statements = json!({
        "rule": { "kind": "function_declaration", "has": { "field": "name", "pattern": "$N" },
            "all": [{ "has": { "field": "body",
                "pattern": { "context": "function f() { $$$S }", "selector": "statement_block" },
                "count": { "has": { "stopBy": "neighbor", "not": { "any": [{ "kind": "comment" }, { "regex": "^[{}]$" }] } }, "min": 2 } } }] },
        "transform": { "C": { "count": { "source": "$$$S" } } },
    });
    assert_eq!(
        messages(
            statements,
            "Function '$N' has too many statements ($C). Maximum allowed is 1.",
            oracle!("ecma/lines/eslint/max-statements@comments.ts")
        ),
        expected_messages("max-statements@comments.ts")
    );
}

/// no-irregular-whitespace (`skipStrings`): a walk over the program's text that consumes strings
/// and names each run outside them, `each` over the group. A program whose runs all sit in strings
/// passes the gate but binds no run: no finding (the fix round after the suites scored M12).
#[test]
fn each_over_a_regex_group_reports_nothing_where_no_match_binds_it() {
    let ast = json!({ "rule": {
        "kind": "program",
        "regex": "[\\x{A0}\\x{3000}]",
        "all": [{ "regex": "'(?:[^'\\\\\\n]|\\\\.)*'|\"(?:[^\"\\\\\\n]|\\\\.)*\"|(?P<WS>[\\x{A0}\\x{3000}]+)" }],
    } });
    for fixture in [
        "no-irregular-whitespace@strings.ts",
        "no-irregular-whitespace@code.ts",
    ] {
        let source = match fixture {
            "no-irregular-whitespace@strings.ts" => {
                oracle!("ecma/lines/eslint/no-irregular-whitespace@strings.ts")
            }
            _ => oracle!("ecma/lines/eslint/no-irregular-whitespace@code.ts"),
        };
        let mut finding = ast.clone();
        finding["language"] = json!("typescript");
        let document = check_document(json!({
            "id": "port",
            "forbid": { "ast": finding },
            "finding": { "anchor": "capture.WS", "code": "PORT", "message": "port",
                "severity": "error", "each": "WS" },
        }));
        let compiled = compile_document_with(
            document,
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "typescript", "fixture.ts", source);
        let mut found = result["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|finding| finding["span"]["startByte"].as_u64().expect("start"))
            .collect::<Vec<_>>();
        found.sort_unstable();
        assert_eq!(found, expected(fixture), "{fixture}");
    }
}

/// capitalized-comments (`never`): the fix lowercases the comment's first letter, a regex group of
/// the witness read by a `convert` transform (the fix round after the suites scored M12; before,
/// a port captured the whole comment and cut the letter out of it with `replace`).
#[test]
fn a_transform_of_a_regex_group_fixes_the_letter() {
    let fixture = "capitalized-comments@fix.ts";
    let source = oracle!("ecma/lines/eslint/capitalized-comments@fix.ts");
    let document = check_document(json!({
        "id": "port",
        "forbid": { "ast": { "language": "typescript",
            "rule": { "kind": "comment", "regex": "^(?://|/\\*)[\\s*]*(?<L>\\p{Lu})" },
            "transform": { "C": { "convert": { "source": "$L", "toCase": "lowerCase" } } } } },
        "finding": { "anchor": "witness.span", "code": "PORT", "message": "port", "severity": "error",
            "fix": { "template": "$C", "range": "capture.L" } },
    }));
    let compiled = compile_document_with(
        document,
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "typescript", "fixture.ts", source);
    let mut fixes = result["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|finding| {
            let fix = &finding["fix"];
            (
                fix["span"]["startByte"].as_u64().expect("start") as usize,
                fix["span"]["endByte"].as_u64().expect("end") as usize,
                fix["text"].as_str().expect("text").to_owned(),
            )
        })
        .collect::<Vec<_>>();
    fixes.sort();
    let mut fixed = source.to_owned();
    for (start, end, text) in fixes.into_iter().rev() {
        fixed.replace_range(start..end, &text);
    }
    let oracle: Value = serde_json::from_str(ORACLE).expect("oracle JSON");
    assert_eq!(
        fixed,
        oracle["fixes"][fixture].as_str().expect("fixed text")
    );
    let mut starts = result["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|finding| finding["span"]["startByte"].as_u64().expect("start"))
        .collect::<Vec<_>>();
    starts.sort_unstable();
    assert_eq!(starts, expected(fixture));
}

/// no-redeclare with `builtinGlobals` over every item of `/* global */` comments (the fix round
/// after the suites scored M12): an item that names a declared global of the query's
/// `languageOptions` (`resolves` on the item: ecmaVersion builtins and configured globals), or,
/// through `host`, one a declaration of the file or an earlier comment already names. `each` over
/// the item: the host tests run per item inside the parsed tree's search, so a later item is
/// reached when an earlier one fails them (`/*global b, a*/ var a;` reports `a`).
#[test]
fn no_redeclare_reports_every_item_a_host_test_or_a_global_names() {
    let fixture = "no-redeclare@items.ts";
    let names = |item: Value| json!({ "grammar": "eslintDirective", "value": { "regex": "^globals?$" }, "has": item });
    let declared = json!({ "inside": { "kind": "program", "has": { "stopBy": "end",
        "any": [{ "kind": "variable_declarator" }, { "kind": "function_declaration" }],
        "has": { "field": "name", "pattern": "$V" },
        "compare": { "left": "V", "op": "eq", "right": "N", "text": "name" } } } });
    let earlier = json!({ "follows": { "stopBy": "end", "kind": "comment", "parse": names(json!({
        "kind": "item", "capture": "M", "not": { "has": { "kind": "setting", "regex": "^off$" } },
        "host": { "compare": { "left": "M", "op": "eq", "right": "N" } } })) } });
    let item = json!({ "kind": "item", "capture": "N",
        "not": { "has": { "kind": "setting", "regex": "^off$" } },
        "any": [{ "resolves": { "global": true } }, { "host": declared }, { "host": earlier }] });
    let document = check_document(json!({
        "id": "port",
        "forbid": { "ast": { "language": "typescript",
            "languageOptions": { "sourceType": "script", "globals": { "conf1": "readonly", "conf2": "writable" } },
            "rule": { "kind": "comment", "parse": names(item) } } },
        "finding": { "anchor": "capture.N", "code": "PORT", "message": "'$N' is already defined.",
            "severity": "error", "each": "N" },
    }));
    let compiled = compile_document_with(
        document,
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let source = oracle!("ecma/lines/eslint/no-redeclare@items.ts");
    let result = evaluate_file(&compiled, "typescript", "fixture.ts", source);
    assert_eq!(result["failures"], json!([]), "{result}");
    let mut found = result["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|finding| finding["span"]["startByte"].as_u64().expect("start"))
        .collect::<Vec<_>>();
    found.sort_unstable();
    found.dedup();
    assert_eq!(found, expected(fixture));
}
