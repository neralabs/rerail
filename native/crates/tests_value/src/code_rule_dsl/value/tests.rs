use super::*;
use crate::code_rule_dsl::language::Family;
use serde_json::Value as Json;

/// Whether a decoded value agrees with an oracle's encoding of the same value: exact for scalars
/// and known sequences; for a container with unknown elements, its kind, and its length and
/// emptiness where known.
pub(crate) fn agrees(value: &Value, expected: &Json) -> bool {
    let Some((tag, payload)) = expected.as_object().and_then(|object| object.iter().next()) else {
        return false;
    };
    let float = |text: &Json| text.as_str().and_then(|text| text.parse::<f64>().ok());
    let same_float = |left: f64, right: Option<f64>| {
        right.is_some_and(|right| {
            (left.is_nan() && right.is_nan()) || left.to_bits() == right.to_bits()
        })
    };
    let container = |kind: ContainerKind, len: Option<usize>| match value {
        Value::Container {
            kind: actual,
            len: actual_len,
            nonempty,
        } => {
            *actual == kind
                && len.is_none_or(|len| {
                    actual_len.is_none_or(|actual| actual == len)
                        && nonempty.is_none_or(|nonempty| nonempty == (len > 0))
                })
        }
        _ => false,
    };
    match (tag.as_str(), value) {
        ("none", Value::None) | ("null", Value::None) => true,
        ("undefined", Value::Undefined) | ("ellipsis", Value::Ellipsis) => true,
        ("bool", Value::Bool(value)) => payload.as_bool() == Some(*value),
        ("int", Value::Int(value)) => payload.as_str() == Some(value.to_string().as_str()),
        ("bigint", Value::BigInt(value)) => payload.as_str() == Some(value.to_string().as_str()),
        ("float", Value::Float(value)) | ("number", Value::Float(value)) => {
            same_float(*value, float(payload))
        }
        ("complex", Value::Complex(real, imag)) => {
            same_float(*real, float(&payload[0])) && same_float(*imag, float(&payload[1]))
        }
        ("str", Value::Str(value)) => payload.as_str() == Some(value.as_str()),
        ("utf16", Value::Utf16(value)) => payload.as_array().is_some_and(|units| {
            units.len() == value.len()
                && units
                    .iter()
                    .zip(value)
                    .all(|(unit, value)| unit.as_u64() == Some(u64::from(*value)))
        }),
        ("bytes", Value::Bytes(value)) => {
            payload.as_str()
                == Some(
                    value
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>()
                        .as_str(),
                )
        }
        ("tuple", Value::Tuple(items)) | ("array", Value::Array(items)) => {
            payload.as_array().is_some_and(|expected| {
                expected.len() == items.len()
                    && items
                        .iter()
                        .zip(expected)
                        .all(|(item, expected)| agrees(item, expected))
            })
        }
        ("tuple", _) => container(ContainerKind::Tuple, payload.as_array().map(Vec::len)),
        ("array", _) => container(ContainerKind::Array, payload.as_array().map(Vec::len)),
        ("list", _) => container(
            ContainerKind::List,
            payload.as_u64().map(|len| len as usize),
        ),
        ("set", _) => container(ContainerKind::Set, payload.as_u64().map(|len| len as usize)),
        ("dict", _) => container(
            ContainerKind::Dict,
            payload.as_u64().map(|len| len as usize),
        ),
        ("object", _) => container(ContainerKind::Object, None),
        ("other", Value::RegExp(_)) => payload == "RegExp",
        _ => false,
    }
}

#[test]
fn python_equality_crosses_numeric_types_and_hash_consistency() {
    let py = |left: Value, right: Value| left.equals(&right, Family::Python);
    assert_eq!(py(Value::int(1), Value::Float(1.0)), Some(true));
    assert_eq!(py(Value::int(1), Value::Bool(true)), Some(true));
    assert_eq!(py(Value::Complex(1.0, 0.0), Value::int(1)), Some(true));
    assert_eq!(py(Value::Float(0.0), Value::Float(-0.0)), Some(true));
    assert_eq!(
        py(Value::Float(f64::NAN), Value::Float(f64::NAN)),
        Some(false)
    );
    // 2**53 + 1 is not the float it rounds to.
    let big = (1i128 << 53) + 1;
    assert_eq!(py(Value::int(big), Value::Float(big as f64)), Some(false));
    assert_eq!(
        py(Value::Str("a".into()), Value::Bytes(b"a".to_vec())),
        Some(false)
    );
    assert_eq!(
        py(
            Value::Tuple(vec![Value::int(1), Value::Float(2.0)]),
            Value::Tuple(vec![Value::Bool(true), Value::int(2)])
        ),
        Some(true)
    );
    let list = |len| Value::Container {
        kind: ContainerKind::List,
        len,
        nonempty: None,
    };
    assert_eq!(py(list(Some(1)), Value::int(1)), Some(false));
    assert_eq!(py(list(Some(1)), list(Some(2))), Some(false));
    assert_eq!(py(list(Some(1)), list(Some(1))), None);
    assert_eq!(py(Value::Unknown, Value::int(1)), None);
}

#[test]
fn javascript_equality_is_same_value() {
    let js = |left: Value, right: Value| left.equals(&right, Family::Ecma);
    assert_eq!(
        js(Value::Float(f64::NAN), Value::Float(f64::NAN)),
        Some(true)
    );
    assert_eq!(js(Value::Float(0.0), Value::Float(-0.0)), Some(false));
    assert_eq!(js(Value::Float(1.0), Value::BigInt(1)), Some(false));
    assert_eq!(js(Value::None, Value::Undefined), Some(false));
    assert_eq!(js(Value::Array(vec![]), Value::Array(vec![])), Some(false));
}

#[test]
fn order_is_numeric_across_types_and_by_code_unit_for_strings() {
    let order = |left: Value, right: Value, lang| left.order(&right, lang);
    assert_eq!(
        order(Value::int(2), Value::Float(2.5), Family::Python),
        Some(Some(Ordering::Less))
    );
    assert_eq!(
        order(
            Value::int(i128::MAX),
            Value::Float(f64::INFINITY),
            Family::Python
        ),
        Some(Some(Ordering::Less))
    );
    assert_eq!(
        order(Value::Float(f64::NAN), Value::int(0), Family::Python),
        Some(None)
    );
    assert_eq!(
        order(Value::Str("a".into()), Value::int(1), Family::Python),
        Some(None)
    );
    // U+FF61 sorts after U+1F600 by code point, before it by UTF-16 code unit.
    assert_eq!(
        order(
            Value::Str("\u{FF61}".into()),
            Value::Str("\u{1F600}".into()),
            Family::Python
        ),
        Some(Some(Ordering::Less))
    );
    let utf16 = |text: &str| Value::Utf16(text.encode_utf16().collect());
    assert_eq!(
        order(utf16("\u{FF61}"), utf16("\u{1F600}"), Family::Ecma),
        Some(Some(Ordering::Greater))
    );
    assert_eq!(
        order(
            Value::Tuple(vec![Value::int(1), Value::int(2)]),
            Value::Tuple(vec![Value::Float(1.0)]),
            Family::Python
        ),
        Some(Some(Ordering::Greater))
    );
    assert_eq!(order(Value::Unknown, Value::int(1), Family::Python), None);
    assert_eq!(
        order(Value::Float(1.0), Value::BigInt(2), Family::Ecma),
        Some(Some(Ordering::Less))
    );
}

#[test]
fn integers_and_floats_compare_exactly() {
    assert_eq!(
        int_float_order(&BigInt::from(3), 2.5),
        Some(Ordering::Greater)
    );
    assert_eq!(
        int_float_order(&BigInt::from(-3), -2.5),
        Some(Ordering::Less)
    );
    assert_eq!(
        int_float_order(&BigInt::from(2), 2.0),
        Some(Ordering::Equal)
    );
    assert_eq!(
        int_float_order(&BigInt::from(i128::MIN), -1.7e38),
        Some(Ordering::Less)
    );
    assert_eq!(
        int_float_order(&BigInt::from(i128::MIN), -1.8e38),
        Some(Ordering::Greater)
    );
    assert_eq!(int_float_order(&BigInt::from(0), f64::NAN), None);
    assert_eq!(
        int_float_order(&BigInt::from(i128::MAX), 1e39),
        Some(Ordering::Less)
    );
}

#[test]
fn types_are_named_as_each_language_names_them() {
    let list = Value::Container {
        kind: ContainerKind::List,
        len: None,
        nonempty: Some(true),
    };
    assert_eq!(list.type_name(Family::Python), Some("list"));
    assert!(list.is_known() && !list.is_constant());
    assert_eq!(Value::None.type_name(Family::Python), Some("NoneType"));
    assert_eq!(Value::None.type_name(Family::Ecma), Some("null"));
    assert_eq!(Value::Float(1.0).type_name(Family::Ecma), Some("number"));
    assert_eq!(Value::Bool(true).type_name(Family::Python), Some("bool"));
    assert_eq!(Value::Unknown.type_name(Family::Python), None);
    assert!(Value::Tuple(vec![]).is_constant());
}

#[test]
fn every_kind_with_a_value_is_an_expression() {
    for kind in python::VALUE_KINDS {
        assert!(python::EXPRESSION_KINDS.contains(kind), "{kind}");
    }
    for kind in ecma::VALUE_KINDS {
        assert!(ecma::EXPRESSION_KINDS.contains(kind), "{kind}");
    }
}
