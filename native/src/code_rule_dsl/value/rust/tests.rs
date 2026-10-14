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
        let root = GrammarDialect::Rust.ast_grep(format!("fn f() {{ let _ = {source}; }}"));
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
            && source
                .ends_with(|c: char| c.is_ascii_digit())
                .then_some(())
                .is_some()
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
        let root = GrammarDialect::Rust.ast_grep(format!("{prefix}fn f() {{ let _ = {source}; }}"));
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
            evaluate_following(&value, &table, &mut Context::new(&mut || true))
        } else {
            // Without items there is no const to follow: both entry points agree.
            let plain = evaluate(&value, &mut Context::new(&mut || true));
            let following = evaluate_following(&value, &table, &mut Context::new(&mut || true));
            // Compared as text: `0.0 / 0.0` is a NaN, which equals nothing.
            if format!("{following:?}") != format!("{plain:?}") {
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
                        && ty.is_none_or(|ty| Some(ty.name()) == case["type"].as_str())
                }
                (Some("float"), RustValue::Float { value, ty }) => {
                    let bits = expected[3].as_str().unwrap_or("");
                    let matches = if expected[1] == "f32" {
                        (*value as f32).to_bits() == u32::from_str_radix(bits, 16).unwrap_or(0)
                    } else {
                        value.to_bits() == u64::from_str_radix(bits, 16).unwrap_or(0)
                    };
                    matches && ty.is_none_or(|ty| Some(ty.name()) == case["type"].as_str())
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
        let unexplained = if known {
            !(exempt && expected_error)
        } else {
            !excused
        };
        if !good && unexplained {
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

/// `evaluate` of the expression `source`, read as a `let` initializer.
fn evaluated(source: &str) -> RustValue {
    let root = GrammarDialect::Rust.ast_grep(format!("fn f() {{ let _ = {source}; }}"));
    let value = root
        .root()
        .dfs()
        .find_map(|node| {
            (node.kind().as_ref() == "let_declaration")
                .then(|| node.field("value"))
                .flatten()
        })
        .expect("a let initializer");
    evaluate(&value, &mut Context::new(&mut || true))
}

fn int(value: &str, ty: IntTy) -> RustValue {
    RustValue::Int {
        value: value.parse().expect("integer"),
        ty: Some(ty),
    }
}

fn f32_bits(value: RustValue) -> u32 {
    match value {
        RustValue::Float {
            value,
            ty: Some(FloatTy::F32),
        } => (value as f32).to_bits(),
        other => panic!("no f32: {other:?}"),
    }
}

#[test]
fn integer_to_f32_casts_round_once_to_f32() {
    // rustc 1.99.0: true, 5d800001, and the f64 detour's 5d800000.
    assert_eq!(
        evaluated("(16777217u32 as f32) == 16777216f32"),
        RustValue::Bool(true)
    );
    assert_eq!(
        f32_bits(evaluated("0x1000_0010_0000_0001u64 as f32")),
        0x5d80_0001
    );
    assert_eq!(
        evaluated("0x1000_0010_0000_0001u64 as f32 == 0x1000_0010_0000_0001u64 as f64 as f32"),
        RustValue::Bool(false)
    );
    assert_eq!(
        f32_bits(evaluated(
            "340282366920938463463374607431768211455u128 as f32"
        )),
        f32::INFINITY.to_bits()
    );
}

#[test]
fn float_to_u128_casts_keep_values_past_i128() {
    // rustc 1.99.0 values.
    assert_eq!(
        evaluated("2e38f64 as u128"),
        int("199999999999999995497619646912068059136", IntTy::U128)
    );
    assert_eq!(
        evaluated("3.5e38f64 as u128"),
        int(&u128::MAX.to_string(), IntTy::U128)
    );
    assert_eq!(
        evaluated("-2e38f64 as i128"),
        int(&i128::MIN.to_string(), IntTy::I128)
    );
    assert_eq!(evaluated("-1.5f64 as u8"), int("0", IntTy::U8));
}

#[test]
fn shifts_keep_the_left_type_and_drop_shifted_out_bits() {
    // rustc 1.99.0: 0, -128, 2, -64, -1; a count past the width is a const error.
    assert_eq!(evaluated("128u8 << 1"), int("0", IntTy::U8));
    assert_eq!(evaluated("64i8 << 1"), int("-128", IntTy::I8));
    assert_eq!(evaluated("1u8 << 1u32"), int("2", IntTy::U8));
    assert_eq!(evaluated("-128i8 >> 1"), int("-64", IntTy::I8));
    assert_eq!(evaluated("-1i8 >> 7"), int("-1", IntTy::I8));
    assert_eq!(evaluated("1u8 << 8"), RustValue::Unknown);
    assert_eq!(evaluated("1u8 >> 8u64"), RustValue::Unknown);
    assert_eq!(evaluated("1u8 << -1i32"), RustValue::Unknown);
}

#[test]
fn unsuffixed_operands_take_their_peers_type() {
    // rustc 1.99.0: true, 3e99999a; `1u8 == 256` and `1u8 + (0 - 1 + 1)` are errors.
    assert_eq!(evaluated("0.1f32 == 0.1"), RustValue::Bool(true));
    assert_eq!(evaluated("0.1 == 0.1f32"), RustValue::Bool(true));
    assert_eq!(f32_bits(evaluated("0.1f32 + 0.2")), 0x3e99_999a);
    assert_eq!(evaluated("1.0f32 + 1"), RustValue::Unknown);
    assert_eq!(evaluated("1u8 == 256"), RustValue::Unknown);
    assert_eq!(evaluated("1u8 + (0 - 1 + 1)"), RustValue::Unknown);
}

/// The `let` initializer of `source` and the steps `evaluate` spent on it.
fn evaluated_counting(source: &str) -> (RustValue, usize) {
    let root = GrammarDialect::Rust.ast_grep(format!("fn f() {{ let _ = {source}; }}"));
    let value = root
        .root()
        .dfs()
        .find_map(|node| {
            (node.kind().as_ref() == "let_declaration")
                .then(|| node.field("value"))
                .flatten()
        })
        .expect("a let initializer");
    let mut steps = 0usize;
    let mut spend = || {
        steps += 1;
        true
    };
    let value = evaluate(&value, &mut Context::new(&mut spend));
    (value, steps)
}

#[test]
fn deep_expressions_are_unknown_without_exhausting_a_small_stack() {
    // A debug build on a 256 KiB thread: the nesting stops at `MAX_DEPTH`.
    std::thread::Builder::new()
        .stack_size(256 << 10)
        .spawn(|| {
            let terms = vec!["1"; 3000].join(" + ");
            assert_eq!(evaluated(&terms), RustValue::Unknown);
            let parens = format!("{}1{}", "(".repeat(2000), ")".repeat(2000));
            assert_eq!(evaluated(&parens), RustValue::Unknown);
            // A right operand nested as deep: its type is read before the left is evaluated.
            assert_eq!(evaluated(&format!("1u8 + {parens}")), RustValue::Unknown);
            assert_eq!(evaluated(&format!("1 + {parens}")), RustValue::Unknown);
            assert_eq!(
                evaluated(&format!("{}1i32", "-".repeat(3000))),
                RustValue::Unknown
            );
            // Within the bound the value is rustc's.
            assert_eq!(
                evaluated(&vec!["1"; 100].join(" + ")),
                RustValue::Int {
                    value: 100.into(),
                    ty: None
                }
            );
            assert_eq!(
                evaluated(&format!("{}1u8{}", "(".repeat(200), ")".repeat(200))),
                int("1", IntTy::U8)
            );
        })
        .expect("thread")
        .join()
        .expect("no stack overflow");
}

#[test]
fn an_operand_is_evaluated_once() {
    // Each `+` gives its unknown left operand the type of its suffixed right one: read from the
    // right operand's syntax, the left is not evaluated again, so the steps grow with the terms.
    for terms in [50usize, 200] {
        let source = format!("a{}", " + 1u8".repeat(terms));
        let (value, steps) = evaluated_counting(&source);
        assert_eq!(value, RustValue::Unknown);
        assert!(steps <= 4 * terms, "{terms} terms: {steps} steps");
    }
    let (value, _) = evaluated_counting(&format!("0{}", " + 1u8".repeat(200)));
    assert_eq!(value, int("200", IntTy::U8));
}

#[test]
fn crlf_line_endings_in_strings_read_as_lf() {
    // rustc 1.99.0 on a CRLF file: "a\nb" for each, "ab" after a continuation; a bare CR is an
    // error in a string and in a raw string.
    let text = |value: RustValue| match value {
        RustValue::Str(text) => text,
        other => panic!("no str: {other:?}"),
    };
    assert_eq!(text(evaluated("\"a\r\nb\"")), "a\nb");
    assert_eq!(text(evaluated("r\"a\r\nb\"")), "a\nb");
    assert_eq!(text(evaluated("r#\"a\r\nb\"#")), "a\nb");
    assert_eq!(text(evaluated("\"a\\\r\n   b\"")), "ab");
    assert_eq!(evaluated("b\"a\r\nb\""), RustValue::Bytes(b"a\nb".to_vec()));
    assert_eq!(
        evaluated("br\"a\r\nb\""),
        RustValue::Bytes(b"a\nb".to_vec())
    );
    assert_eq!(evaluated("\"a\rb\""), RustValue::Unknown);
    assert_eq!(evaluated("r\"a\rb\""), RustValue::Unknown);
}
