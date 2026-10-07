//! Ruff-shaped ports of Pyflakes' format rules (F501-F509, F521-F525), against where Ruff 0.16.9
//! reports them on its own fixtures (`fixtures/python/literal/ruff`, `fixtures/python/literal/generate_ruff.py`).

use crate::policy_dsl::oracle;
use crate::policy_dsl::test_support::*;
use serde_json::{Value, json};

const ORACLE: &str = oracle!("python/literal/ruff.expected.json");

/// A string literal Ruff's `Expr::StringLiteral` is: no `f`, `b` or `t` prefix in any part.
fn plain_string() -> Value {
    let prefixed = "^[A-Za-z]*[fFbBtT][A-Za-z]*['\"]";
    json!({ "any": [
        { "kind": "string", "not": { "regex": prefixed } },
        { "kind": "concatenated_string", "not": { "has": { "kind": "string", "regex": prefixed } } },
    ] })
}

/// `$S % $R` with a plain string on the left, whose parsed format satisfies `format`.
fn percent(format: Value, extra: Value) -> Value {
    let mut rule = json!({ "pattern": "$S % $R" });
    if let Value::Object(extra) = extra {
        rule.as_object_mut().expect("rule").extend(extra);
    }
    json!({
        "rule": rule,
        "utils": { "plain-string": plain_string() },
        "constraints": { "S": { "matches": "plain-string", "parse": format } },
    })
}

/// `$S.format($$$A)` with a plain string, whose parsed template satisfies `template`.
fn brace(template: Value, extra: Value) -> Value {
    let mut rule = json!({ "pattern": "$S.format($$$A)" });
    if let Value::Object(extra) = extra {
        rule.as_object_mut().expect("rule").extend(extra);
    }
    json!({
        "rule": rule,
        "utils": { "plain-string": plain_string() },
        "constraints": { "S": { "matches": "plain-string", "parse": template } },
    })
}

/// `$S % $R` with `formatArgs`, which reads `S` as Ruff does (through parentheses, a plain `str`
/// literal whose format it accepts) and needs no constraint on it.
fn percent_args(test: Value) -> Value {
    let mut test = test;
    test["style"] = json!("percent");
    test["string"] = json!("S");
    test["args"] = json!("R");
    json!({ "rule": { "pattern": "$S % $R", "formatArgs": test } })
}

/// `$S.format($$$A)` with `formatArgs`, as `percent_args`.
fn brace_args(test: Value) -> Value {
    let mut test = test;
    test["style"] = json!("brace");
    test["string"] = json!("S");
    test["args"] = json!("A");
    json!({ "rule": { "pattern": "$S.format($$$A)", "formatArgs": test } })
}

/// Ruff's `keywords` is not empty: a placeholder with a mapping key.
fn keyed() -> Value {
    json!({ "kind": "placeholder", "stopBy": "end", "has": { "kind": "mapping_key" } })
}

/// A value the format consumes by position: an unkeyed placeholder or a star.
fn positional() -> Value {
    json!({ "stopBy": "end", "any": [
        { "kind": "placeholder", "not": { "has": { "kind": "mapping_key" } } },
        { "kind": "star" },
    ] })
}

/// The right operand, parentheses aside, is one of these kinds.
fn operand(kinds: &[&str]) -> Value {
    let kinds: Vec<Value> = kinds.iter().map(|kind| json!({ "kind": kind })).collect();
    json!({ "any": [
        { "any": kinds.clone() },
        { "kind": "parenthesized_expression", "has": { "any": kinds } },
    ] })
}

fn port(code: &str) -> Value {
    match code {
        "F501" => percent(
            json!({ "grammar": "percentFormat", "has": { "kind": "error", "not": { "value": { "regex": "^unsupported format character" } } } }),
            json!({}),
        ),
        "F509" => percent(
            json!({ "grammar": "percentFormat", "has": { "kind": "error", "value": { "regex": "^unsupported format character" } } }),
            json!({}),
        ),
        "F502" => {
            let mut port = percent(
                json!({ "grammar": "percentFormat", "has": keyed() }),
                json!({}),
            );
            port["constraints"]["R"] = operand(&[
                "list",
                "tuple",
                "set",
                "list_comprehension",
                "set_comprehension",
                "generator_expression",
            ]);
            port
        }
        "F503" => {
            let mut port = percent(
                json!({ "grammar": "percentFormat", "count": { "has": positional(), "min": 2 } }),
                json!({}),
            );
            port["constraints"]["R"] = operand(&["dictionary", "dictionary_comprehension"]);
            port
        }
        "F504" => percent_args(json!({ "unused": { "kind": "pair" } })),
        "F505" => percent_args(
            json!({ "missing": { "kind": "placeholder", "has": { "kind": "mapping_key" } } }),
        ),
        "F506" => percent(
            json!({ "grammar": "percentFormat", "all": [{ "has": keyed() }, { "has": positional() }] }),
            json!({}),
        ),
        "F507" => {
            let mut port = percent_args(json!({ "unused": { "not": { "kind": "pair" } } }));
            let missing = percent_args(json!({ "missing": { "any": [{ "kind": "star" },
                { "kind": "placeholder", "not": { "has": { "kind": "mapping_key" } } }] } }));
            port["rule"] = json!({ "any": [port["rule"], missing["rule"]] });
            port
        }
        "F508" => {
            let mut port = percent(
                json!({ "grammar": "percentFormat", "has": { "kind": "star", "stopBy": "end" } }),
                json!({}),
            );
            port["constraints"]["R"] = operand(&["dictionary", "dictionary_comprehension"]);
            port
        }
        "F521" => brace(
            json!({ "grammar": "braceFormat", "has": { "kind": "error" } }),
            json!({}),
        ),
        "F522" => brace_args(json!({ "unused": { "kind": "keyword_argument" } })),
        "F523" => brace_args(json!({ "unused": { "not": { "kind": "keyword_argument" } } })),
        "F524" => brace_args(json!({ "missing": { "kind": "field" } })),
        "F525" => brace(
            json!({ "grammar": "braceFormat", "all": [
                { "has": { "kind": "auto", "stopBy": "end" } },
                { "has": { "kind": "index", "stopBy": "end" } },
            ] }),
            json!({}),
        ),
        other => panic!("no port for {other}"),
    }
}

/// Where a Python query reports on a source: each finding's byte range, sorted.
fn ranges(ast: Value, source: &str) -> Vec<[u64; 2]> {
    let mut ast = ast;
    ast["language"] = json!("python");
    let document =
        check_document(json!({ "id": "port", "forbid": { "ast": ast }, "violation": finding() }));
    let compiled = compile_document_with(
        document,
        &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "python", "fixture.py", source);
    assert_eq!(result["failures"], json!([]), "{result}");
    let mut ranges: Vec<[u64; 2]> = result["violations"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|finding| {
            [
                finding["span"]["startByte"].as_u64().expect("start"),
                finding["span"]["endByte"].as_u64().expect("end"),
            ]
        })
        .collect();
    ranges.sort_unstable();
    ranges
}

#[test]
fn ruff_format_rules_report_exactly_ruffs_ranges() {
    let oracle: Value = serde_json::from_str(ORACLE).expect("oracle");
    let fixtures = [
        ("F502.py", oracle!("python/literal/ruff/F502.py")),
        ("F503.py", oracle!("python/literal/ruff/F503.py")),
        ("F504.py", oracle!("python/literal/ruff/F504.py")),
        ("F50x.py", oracle!("python/literal/ruff/F50x.py")),
        (
            "F50x_operands.py",
            oracle!("python/literal/ruff/F50x_operands.py"),
        ),
        (
            "F50x_parenthesized.py",
            oracle!("python/literal/ruff/F50x_parenthesized.py"),
        ),
        ("F521.py", oracle!("python/literal/ruff/F521.py")),
        ("F522.py", oracle!("python/literal/ruff/F522.py")),
        ("F523.py", oracle!("python/literal/ruff/F523.py")),
        ("F524.py", oracle!("python/literal/ruff/F524.py")),
        ("F525.py", oracle!("python/literal/ruff/F525.py")),
        (
            "F52x_parenthesized.py",
            oracle!("python/literal/ruff/F52x_parenthesized.py"),
        ),
    ];
    let mut mismatches = Vec::new();
    for (name, source) in fixtures {
        for (code, expected) in oracle[name].as_object().expect("codes") {
            let expected: Vec<[u64; 2]> = expected
                .as_array()
                .expect("ranges")
                .iter()
                .map(|range| {
                    [
                        range[0].as_u64().expect("start"),
                        range[1].as_u64().expect("end"),
                    ]
                })
                .collect();
            let found = ranges(port(code), source);
            if found != expected {
                let text = |ranges: &[[u64; 2]]| {
                    ranges
                        .iter()
                        .map(|[start, end]| {
                            let line = source[..*start as usize].matches('\n').count() + 1;
                            format!("{line}:{}", &source[*start as usize..*end as usize])
                        })
                        .collect::<Vec<_>>()
                };
                mismatches.push(format!(
                    "{name} {code}: Rerail {:?}, Ruff {:?}",
                    text(&found),
                    text(&expected)
                ));
            }
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}
