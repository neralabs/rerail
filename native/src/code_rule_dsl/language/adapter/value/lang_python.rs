//! Moved from `value.rs` by the crate split (tools/cratify/moves.tsv).

use super::*;
use crate::code_rule_dsl::language::adapter::python::Python;

impl ValueRules for Python {
    const SUPPORTED: bool = true;
    const LANG: Family = Family::Python;
    const EXPRESSION_KINDS: &'static [&'static str] = value::python::EXPRESSION_KINDS;
    const VALUE_KINDS: &'static [&'static str] = value::python::VALUE_KINDS;
    const NAME_KINDS: &'static [&'static str] = &["identifier"];

    /// NaN is truthy.
    fn truthy(value: &Value) -> Option<bool> {
        value.truthy_where(true)
    }

    fn type_name(value: &Value) -> Option<&'static str> {
        value.type_name_as(&TypeNames {
            none: "NoneType",
            bool: "bool",
            float: "float",
        })
    }

    fn equals(left: &Value, right: &Value) -> Option<bool> {
        value::python_equals(left, right)
    }

    /// `bool` is an `int`; tuples order element by element.
    fn order(left: &Value, right: &Value) -> Option<Option<Ordering>> {
        left.ordered(
            right,
            Ordered {
                bools_are_ints: true,
                tuples: true,
            },
        )
    }

    /// Values that do not order (NaN, different types) fail an ordering.
    fn relation(operator: &str, left: &Value, right: &Value) -> Option<bool> {
        Self::order(left, right).map(|order| {
            order.is_some_and(|order| match operator {
                "<" => order == Ordering::Less,
                "<=" => order != Ordering::Greater,
                ">" => order == Ordering::Greater,
                _ => order != Ordering::Less,
            })
        })
    }

    fn numeric(value: &Value) -> bool {
        matches!(
            value,
            Value::Int(_) | Value::Float(_) | Value::BigInt(_) | Value::Bool(_)
        )
    }

    fn integer(value: &Value) -> Option<BigInt> {
        match value {
            Value::Int(value) => Some(value.clone()),
            Value::BigInt(value) => Some(BigInt::from(*value)),
            Value::Bool(value) => Some(BigInt::from(u8::from(*value))),
            _ => None,
        }
    }

    /// `str(v)` of a scalar.
    fn to_str<D: Doc>(value: &Value, _: &Node<'_, D>) -> Option<String> {
        value::python::to_str(value)
    }

    fn decode<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Value {
        value::python::decode(node, context)
    }

    /// Ruff's equalities.
    fn structure<D: Doc>(
        node: &Node<'_, D>,
        equality: Equality,
        context: &mut Context<'_>,
    ) -> Option<Vec<Tok>> {
        crate::code_rule_dsl::value::python_structure::structure(node, equality, context)
    }

    fn string(text: &str) -> Value {
        Value::Str(text.to_owned().into())
    }

    /// A JSON integer is an `int`.
    fn number(number: &serde_json::Number) -> Value {
        match (number.as_i64(), number.as_u64()) {
            (Some(integer), _) => Value::int(integer),
            (None, Some(integer)) => Value::int(integer),
            (None, None) => Value::Float(number.as_f64().unwrap_or(f64::NAN)),
        }
    }

    fn spelled<D: Doc>(_: &Node<'_, D>) -> bool {
        true
    }

    fn node_name<D: Doc>(_: &Node<'_, D>) -> Option<Vec<u16>> {
        unreachable!("`name` is refused for this language when the policy compiles")
    }

    fn key_name<'r, D: Doc>(
        _: &Node<'r, D>,
        _: &mut dyn FnMut(&Node<'r, D>) -> Value,
    ) -> Option<Vec<u16>> {
        unreachable!("`key` is refused for this language when the policy compiles")
    }

    fn member_name<'r, D: Doc>(
        _: &Node<'r, D>,
        _: &mut dyn FnMut(&Node<'r, D>) -> Value,
    ) -> Option<Vec<u16>> {
        unreachable!("`member` is refused for this language when the policy compiles")
    }

    fn same_reference<D: Doc>(_: &Node<'_, D>, _: &Node<'_, D>, _: bool) -> Option<bool> {
        unreachable!("`reference` is refused for this language when the policy compiles")
    }

    fn feature(feature: ValueFeature<'_>) -> Result<(), Refusal> {
        match feature {
            ValueFeature::Type(name) => type_name(name, PYTHON_TYPES, "Python"),
            ValueFeature::ToPrecision => no_to_precision("Python"),
            ValueFeature::Compare(text, name) => match text {
                SemanticText::Key | SemanticText::Member => Err(Refusal::Refused(
                    format!("`{name}` reads JavaScript and PHP property names; Python has none")
                        .into(),
                )),
                SemanticText::Reference { .. } => Err(Refusal::Refused(
                    format!(
                        "`{name}` reads JavaScript and PHP references (ESLint's isSameReference); use structure for Python"
                    )
                    .into(),
                )),
                SemanticText::Name => Err(Refusal::Refused(
                    "`name` reads the names JavaScript and PHP identifiers and strings write; use unquoted for Python"
                        .into(),
                )),
                SemanticText::Value | SemanticText::Structure(_) => Ok(()),
            },
        }
    }
}
