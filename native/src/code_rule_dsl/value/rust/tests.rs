use super::*;
use crate::code_rule_dsl::language::GrammarDialect;
use ast_grep_core::tree_sitter::LanguageExt;

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn upstream_literal_oracle() {
    let oracle: serde_json::Value = serde_json::from_str(crate::code_rule_dsl::oracle!(
        "rust/literal/literal.expected.json"
    ))
    .expect("oracle JSON");
    let mut checked = 0usize;
    let mut failures = Vec::new();
    let mut grammar_gaps = Vec::new();
    for case in oracle["cases"].as_array().expect("cases") {
        let Some(source) = case["source"].as_str() else {
            continue;
        };
        let root = GrammarDialect::Rust.ast_grep(&format!("fn f() {{ let _ = {source}; }}"));
        let node = root.root().dfs().find(|n| {
            matches!(
                n.kind().as_ref(),
                "integer_literal"
                    | "float_literal"
                    | "char_literal"
                    | "string_literal"
                    | "raw_string_literal"
                    | "byte_literal"
                    | "byte_string_literal"
                    | "raw_byte_string_literal"
                    | "c_string_literal"
                    | "raw_c_string_literal"
                    | "boolean_literal"
            )
        });
        // `concat!`/`stringify!` are macros: their value is no literal's.
        if source.contains('!') {
            continue;
        }
        let accepted = case
            .get("accepted")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        // rustc rejects an unsuffixed literal out of range of the type it infers (`i32` alone);
        // in other contexts the same text is another type's value.
        let unsuffixed_overflow = !accepted
            && case["diagnostics"][0]["code"].as_str() == Some("overflowing_literals")
            && !source
                .ends_with(|c: char| c.is_ascii_digit())
                .then_some(())
                .is_none()
            && source
                .chars()
                .all(|c| c.is_ascii_digit() || c == '_' || c == '.' || c == 'e');
        if unsuffixed_overflow {
            continue;
        }
        // tree-sitter-rust 0.24.2's token rules refuse `_` inside a `\u{..}` escape, which rustc
        // accepts: such a file has a syntax error, and the engine reads none of its literals.
        if root.root().dfs().any(|n| n.is_error()) {
            if accepted {
                grammar_gaps.push(case["name"].as_str().unwrap_or_default().to_owned());
            }
            continue;
        }
        // A literal the grammar splits into several tokens (`0b102`) is not one literal node:
        // the engine never decodes it as the whole text.
        let value = match node {
            Some(node) if node.text() == source => literal(&node),
            _ => RustValue::Unknown,
        };
        checked += 1;
        let good = if !accepted {
            value == RustValue::Unknown
        } else {
            match case["kind"].as_str() {
                Some("int") => {
                    matches!((&value, case["value"].as_str()), (RustValue::Int { value, .. }, Some(expected)) if value.to_string() == expected)
                }
                Some("float") => match (&value, case["bits"].as_str(), case["type"].as_str()) {
                    (RustValue::Float { value, .. }, Some(bits), Some("f32")) => {
                        (*value as f32).to_bits() == u32::from_str_radix(bits, 16).unwrap_or(0)
                    }
                    (RustValue::Float { value, .. }, Some(bits), _) => {
                        value.to_bits() == u64::from_str_radix(bits, 16).unwrap_or(0)
                    }
                    _ => false,
                },
                Some("char") => {
                    matches!((&value, case["value"].as_str()), (RustValue::Char(actual), Some(expected)) if actual.to_string() == expected)
                }
                Some("str") => {
                    matches!((&value, case["value"].as_str()), (RustValue::Str(actual), Some(expected)) if actual == expected)
                }
                Some("bytes") => {
                    matches!((&value, case["bytes"].as_str()), (RustValue::Bytes(actual), Some(expected)) if encode_hex(actual) == expected)
                }
                Some("cstr") => {
                    matches!((&value, case["bytes"].as_str()), (RustValue::CStr(actual), Some(expected)) if encode_hex(actual) == expected)
                }
                Some("bool") => {
                    matches!((&value, case["value"].as_bool()), (RustValue::Bool(actual), Some(expected)) if actual == &expected)
                }
                _ => true,
            }
        };
        if !good {
            failures.push(format!("{}: {:?} vs {}", case["name"], value, case));
        }
    }
    assert!(
        failures.is_empty(),
        "oracle failures (checked {checked}):\n{}",
        failures.join("\n")
    );
    assert!(checked > 100, "only {checked} oracle cases checked");
    assert_eq!(
        grammar_gaps,
        ["char_escape_unicode_separators", "str_escape_unicode"],
        "literals rustc accepts that tree-sitter-rust does not parse"
    );
}

#[test]
fn upstream_value_oracle() {
    let oracle: serde_json::Value = serde_json::from_str(crate::code_rule_dsl::oracle!(
        "rust/value/rust.expected.json"
    ))
    .expect("oracle JSON");
    const EXEMPT: &[(&str, &str)] = &[
        (
            "!0",
            "unsuffixed integer type comes from expression context",
        ),
        (
            "!2",
            "unsuffixed integer type comes from expression context",
        ),
        ("1-2", "overflow verdict depends on inferred integer type"),
        (
            "1<<8",
            "shift overflow verdict depends on inferred integer type",
        ),
        (
            "1>>8",
            "shift overflow verdict depends on inferred integer type",
        ),
        (
            "1<<32",
            "shift overflow verdict depends on inferred integer type",
        ),
        (
            "1>>32",
            "shift overflow verdict depends on inferred integer type",
        ),
        (
            "255&15",
            "literal range verdict depends on inferred integer type",
        ),
        (
            "255^15",
            "literal range verdict depends on inferred integer type",
        ),
        (
            "300 as u8",
            "rustc infers the cast operand type before rejecting it",
        ),
        (
            "{ let x = 2; x + 3 }",
            "local bindings are outside expression-only folding",
        ),
        ("[1, 2][0]", "indexing is explicitly Unknown"),
        ("(1, 2).0", "tuple field access is explicitly Unknown"),
    ];
    let mut checked = 0usize;
    let mut followed = 0usize;
    let mut failures = Vec::new();
    for case in oracle["cases"].as_array().expect("cases") {
        let source = case["src"].as_str().expect("src");
        // `items` are module-level items the expression may name; the probe follows them.
        let items = case.get("items").and_then(serde_json::Value::as_str);
        let prefix = items.map_or_else(String::new, |items| format!("{items}\n"));
        let root =
            GrammarDialect::Rust.ast_grep(&format!("{prefix}fn f() {{ let _ = {source}; }}"));
        if root.root().dfs().any(|node| node.is_error()) {
            continue;
        }
        let Some(value) = root.root().dfs().find_map(|node| {
            (node.kind().as_ref() == "let_declaration" && node.range().start >= prefix.len())
                .then(|| node.field("value"))
                .flatten()
        }) else {
            continue;
        };
        checked += 1;
        let expected = case
            .get("const")
            .filter(|value| value[0] != "error")
            .unwrap_or(&case["eval"]);
        let table = crate::code_rule_dsl::scope::rust::build_for_test(&format!(
            "{prefix}fn f() {{ let _ = {source}; }}"
        ));
        let actual = if items.is_some() {
            followed += 1;
            evaluate_following(&value, &table)
        } else {
            // Without items there is no const to follow: both entry points agree.
            let plain = evaluate(&value);
            // Compared as text: `0.0 / 0.0` is a NaN, which equals nothing.
            if format!("{:?}", evaluate_following(&value, &table)) != format!("{plain:?}") {
                failures.push(format!(
                    "{source}: evaluate_following differs from evaluate"
                ));
            }
            plain
        };
        let known = !matches!(actual, RustValue::Unknown);
        let expected_error = expected[0] == "error";
        let good = if expected_error {
            !known
        } else {
            match (expected[0].as_str(), &actual) {
                (Some("int"), RustValue::Int { value, ty }) => {
                    value.to_string() == expected[2].as_str().unwrap_or("")
                        && ty.map_or(true, |ty| Some(ty.name()) == case["type"].as_str())
                }
                (Some("float"), RustValue::Float { value, ty }) => {
                    let bits = expected[3].as_str().unwrap_or("");
                    let matches = if expected[1] == "f32" {
                        (*value as f32).to_bits() == u32::from_str_radix(bits, 16).unwrap_or(0)
                    } else {
                        value.to_bits() == u64::from_str_radix(bits, 16).unwrap_or(0)
                    };
                    matches && ty.map_or(true, |ty| Some(ty.name()) == case["type"].as_str())
                }
                (Some("bool"), RustValue::Bool(value)) => Some(*value) == expected[1].as_bool(),
                (Some("char"), RustValue::Char(value)) => {
                    value.to_string() == expected[1].as_str().unwrap_or("")
                }
                (Some("str"), RustValue::Str(value)) => value == expected[1].as_str().unwrap_or(""),
                (Some("bytes"), RustValue::Bytes(value)) => {
                    encode_hex(value) == expected[2].as_str().unwrap_or("")
                }
                _ => false,
            }
        };
        // A value Rerail knows must be rustc's, but where rustc rejects the program only for the
        // type its context gives an unsuffixed expression (EXEMPT): Rerail reads the expression
        // alone, the value it has in every context that compiles. One it does not know must be
        // one the case or EXEMPT says a reader of the expression alone cannot know.
        let exempt = EXEMPT.iter().any(|(src, _)| *src == source);
        let excused = case.get("unknown").is_some() || exempt;
        if !good && ((known && !(exempt && expected_error)) || (!known && !excused)) {
            failures.push(format!("{source}: {actual:?} vs {expected}"));
        }
    }
    assert!(
        failures.is_empty(),
        "oracle failures (checked {checked}):\n{}",
        failures.join("\n")
    );
    assert!(checked >= 200, "only {checked} oracle cases checked");
    assert!(
        followed >= 30,
        "only {followed} const-following cases checked"
    );
}
