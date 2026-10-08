use super::*;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::oracle;
use crate::code_rule_dsl::value::tests::agrees;
use ast_grep_core::tree_sitter::LanguageExt;
use serde_json::Value as Json;

const ORACLE: &str = oracle!("python/value/python.expected.json");

/// The value of the one expression statement `source` holds, and its root kind.
fn decode_kind(source: &str) -> (Value, String) {
    let root = GrammarDialect::Python.ast_grep(source);
    let statement = root
        .root()
        .children()
        .find(|child| child.kind() == "expression_statement")
        .unwrap_or_else(|| panic!("{source:?} is an expression statement"));
    let expression = statement
        .children()
        .find(|child| child.is_named())
        .expect("an expression");
    let kind = expression.kind().to_string();
    (decode_with(source, &mut || true).0, kind)
}

/// The value of the one expression statement `source` holds, decoded with `spend` as the budget.
fn decode_with(source: &str, spend: &mut dyn FnMut() -> bool) -> (Value, bool) {
    let root = GrammarDialect::Python.ast_grep(source);
    let statement = root
        .root()
        .children()
        .find(|child| child.kind() == "expression_statement")
        .unwrap_or_else(|| panic!("{source:?} is an expression statement"));
    let expression = statement
        .children()
        .find(|child| child.is_named())
        .expect("an expression");
    // The cases bind no builtin name, so every builtin is unshadowed.
    let mut global = |_: std::ops::Range<usize>, _: &str| Some(true);
    let mut context = Context::with_globals(spend, &mut global);
    let value = decode(&expression, &mut context);
    (value, context.exhausted())
}

pub(crate) fn decode_source(source: &str) -> Value {
    decode_with(source, &mut || true).0
}

/// Every case in the CPython oracle (`rerail-fixtures/python/value/generate_python.py`):
/// - a value Rerail knows is the value CPython evaluates the expression to;
/// - where CPython evaluates it to nothing (a syntax error), Rerail knows nothing;
/// - where `ast.literal_eval` reads a value, or `optimize=1` folds one, Rerail knows it too, unless
///   the case says why not.
#[test]
fn python_literals_decode_as_cpython_reads_them() {
    let oracle: Json = serde_json::from_str(ORACLE).expect("oracle JSON");
    assert_eq!(oracle["python"].as_str().map(|v| &v[..4]), Some("3.13"));
    let mut known = 0;
    for case in oracle["cases"].as_array().expect("cases") {
        let source = case["src"].as_str().expect("src");
        let value = decode_source(source);
        let evaluated = &case["eval"];
        if value.is_known() {
            known += 1;
            assert!(
                agrees(&value, evaluated),
                "{source:?}: Rerail reads {value:?}, CPython evaluates {evaluated}"
            );
        }
        let literal = case["literalEval"].get("error").is_none();
        if literal && case.get("unknown").is_none() {
            assert!(
                value.is_known(),
                "{source:?}: ast.literal_eval reads {}, Rerail knows nothing",
                case["literalEval"]
            );
        }
        if !case["folded"].is_null() && case.get("unknown").is_none() {
            assert!(
                value.is_known(),
                "{source:?}: CPython folds it to {}, Rerail knows nothing",
                case["folded"]
            );
        }
        if let Some(reason) = case.get("unknown") {
            assert!(
                !value.is_known(),
                "{source:?} is unknown by design: {reason}"
            );
        }
    }
    assert!(known > 200, "{known} cases known");
}

#[test]
fn decoding_stops_at_its_depth_and_step_limits() {
    let nested = format!("{}1{}", "(".repeat(300), ")".repeat(300));
    assert_eq!(decode_source(&nested), Value::Unknown);
    let shallow = format!("{}1{}", "(".repeat(100), ")".repeat(100));
    assert_eq!(decode_source(&shallow), Value::int(1));
    // A tuple of three spends four steps: with two, even its shape is not known.
    let mut left = 2;
    let (value, exhausted) = decode_with("(1, 2, 3)\n", &mut || {
        left -= 1;
        left >= 0
    });
    assert_eq!(value, Value::Unknown);
    assert!(exhausted);
    let mut left = 4;
    let (value, exhausted) = decode_with("(1, 2, 3)\n", &mut || {
        left -= 1;
        left >= 0
    });
    assert!(matches!(value, Value::Tuple(_)), "{value:?}");
    assert!(!exhausted);
}

#[test]
fn a_huge_repetition_is_unknown_before_it_allocates() {
    assert_eq!(decode_source("'a' * 10**9\n"), Value::Unknown);
    assert_eq!(decode_source("(1,) * 10**9\n"), Value::Unknown);
    assert_eq!(
        decode_source("b'ab' * 32768\n"),
        Value::Bytes(b"ab".repeat(32768))
    );
    assert_eq!(decode_source("b'ab' * 32769\n"), Value::Unknown);
}

#[test]
fn an_int_past_the_cap_is_unknown_before_it_is_computed() {
    let started = std::time::Instant::now();
    for source in [
        "2 ** 10**9\n",
        "7 ** 4000\n",
        "1 << 10**18\n",
        "(2 ** 4000) * (2 ** 4000)\n",
        &format!("{}\n", "9".repeat(1_000_000)),
        &format!("0x{}\n", "f".repeat(1_000_000)),
    ] {
        assert_eq!(
            decode_source(source),
            Value::Unknown,
            "{}",
            &source[..20.min(source.len())]
        );
    }
    assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
    // Leading zeros are no bits: a zero-padded literal within the cap is known.
    let padded = format!("0x{}1\n", "0".repeat(5000));
    assert_eq!(decode_source(&padded), Value::int(1));
    // Exactly at the cap, and one past it.
    assert!(matches!(decode_source("-(2 ** 4095)\n"), Value::Int(_)));
    assert!(matches!(
        decode_source("(2 ** 4095) + (2 ** 4095)\n"),
        Value::Unknown
    ));
}

#[test]
fn a_shadowed_builtin_is_not_called() {
    let root = GrammarDialect::Python.ast_grep("list()\n");
    let call = root
        .root()
        .children()
        .next()
        .and_then(|statement| statement.children().next())
        .expect("a call");
    let mut spend = || true;
    let mut shadowed = |_: std::ops::Range<usize>, _: &str| Some(false);
    let mut context = Context::with_globals(&mut spend, &mut shadowed);
    assert_eq!(decode(&call, &mut context), Value::Unknown);
    let mut spend = || true;
    assert_eq!(decode(&call, &mut Context::new(&mut spend)), Value::Unknown);
}

/// Every kind `decode` knows a value for is listed for the anchor plan: a known value's root kind
/// is in `VALUE_KINDS`.
#[test]
fn value_kinds_cover_every_decoded_root() {
    let oracle: Json = serde_json::from_str(ORACLE).expect("oracle JSON");
    for case in oracle["cases"].as_array().expect("cases") {
        let source = case["src"].as_str().expect("src");
        let (value, kind) = decode_kind(source);
        if value.is_known() {
            assert!(
                VALUE_KINDS.contains(&kind.as_str()),
                "{source:?} has kind {kind}"
            );
        }
    }
}

/// `float_repr` against CPython 3.13's `repr`.
#[test]
fn floats_read_as_cpython_reprs_them() {
    use super::float_repr;
    for (value, expected) in [
        (0.1, "0.1"),
        (1e16, "1e+16"),
        (1e15, "1000000000000000.0"),
        (1e-5, "1e-05"),
        (0.0001, "0.0001"),
        (123.456, "123.456"),
        (1.0 / 3.0, "0.3333333333333333"),
        (2.5e-7, "2.5e-07"),
        (1.5e300, "1.5e+300"),
        (-0.0, "-0.0"),
        (f64::INFINITY, "inf"),
        (f64::NEG_INFINITY, "-inf"),
        (f64::NAN, "nan"),
        (5e-324, "5e-324"),
        (100.0, "100.0"),
        (1e22, "1e+22"),
        (12345678901234567.0, "1.2345678901234568e+16"),
        (0.1 + 0.2, "0.30000000000000004"),
        (-1.5, "-1.5"),
    ] {
        assert_eq!(float_repr(value), expected, "{value}");
    }
}
