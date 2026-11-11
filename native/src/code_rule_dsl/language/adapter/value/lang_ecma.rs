//! Moved from `value.rs` by the crate split (tools/cratify/moves.tsv).

use super::*;

impl ValueRules for Ecma {
    const SUPPORTED: bool = true;
    const LANG: Family = Family::Ecma;
    const EXPRESSION_KINDS: &'static [&'static str] = value::ecma::EXPRESSION_KINDS;
    const VALUE_KINDS: &'static [&'static str] = value::ecma::VALUE_KINDS;
    const NAME_KINDS: &'static [&'static str] = &["identifier", "undefined"];

    /// NaN is falsy.
    fn truthy(value: &Value) -> Option<bool> {
        value.truthy_where(false)
    }

    fn type_name(value: &Value) -> Option<&'static str> {
        value.type_name_as(&TypeNames {
            none: "null",
            bool: "boolean",
            float: "number",
        })
    }

    fn equals(left: &Value, right: &Value) -> Option<bool> {
        value::js_same_value(left, right)
    }

    fn order(left: &Value, right: &Value) -> Option<Option<Ordering>> {
        left.ordered(
            right,
            Ordered {
                bools_are_ints: false,
                tuples: false,
            },
        )
    }

    /// As JavaScript's relational operators, converting primitives.
    fn relation(operator: &str, left: &Value, right: &Value) -> Option<bool> {
        value::ecma::ops::relation(operator, left, right)
    }

    fn numeric(value: &Value) -> bool {
        matches!(value, Value::Int(_) | Value::Float(_) | Value::BigInt(_))
    }

    /// A number's bits where it is a safe integer.
    fn integer(value: &Value) -> Option<BigInt> {
        match value {
            Value::Int(value) => Some(value.clone()),
            Value::BigInt(value) => Some(BigInt::from(*value)),
            Value::Float(value)
                if value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_992.0 =>
            {
                Some(BigInt::from(*value as i64))
            }
            _ => None,
        }
    }

    /// `String(v)` of a primitive or a RegExp; an identifier otherwise reads as its name,
    /// escapes decoded.
    fn to_str<D: Doc>(value: &Value, node: &Node<'_, D>) -> Option<String> {
        value::ecma::ops::to_string(value)
            .or_else(|| {
                (!value.is_known() && node.kind() != "string")
                    .then(|| key::node_name(node))
                    .flatten()
            })
            .and_then(|units| String::from_utf16(&units).ok())
    }

    fn decode<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Value {
        value::ecma::decode(node, context)
    }

    fn structure<D: Doc>(_: &Node<'_, D>, _: Equality, _: &mut Context<'_>) -> Option<Vec<Tok>> {
        unreachable!("the structural modes are refused for this language when the policy compiles")
    }

    /// UTF-16 code units.
    fn string(text: &str) -> Value {
        Value::Utf16(text.encode_utf16().collect())
    }

    /// A number.
    fn number(number: &serde_json::Number) -> Value {
        Value::Float(number.as_f64().unwrap_or(f64::NAN))
    }

    /// Whether no token of the node is one an embedder wrote (a Vue view's `(` and `);` around an
    /// interpolation, a character reference's character), which a comparison of tokens must not
    /// read as the file's (`embed::rewritten`).
    fn spelled<D: Doc>(node: &Node<'_, D>) -> bool {
        !crate::code_rule_dsl::embed::rewritten(node)
    }

    /// ESTree's name.
    fn node_name<D: Doc>(node: &Node<'_, D>) -> Option<Vec<u16>> {
        key::node_name(node)
    }

    /// ESLint's `getStaticPropertyName`.
    fn key_name<'r, D: Doc>(
        node: &Node<'r, D>,
        _: &mut dyn FnMut(&Node<'r, D>) -> Value,
    ) -> Option<Vec<u16>> {
        key::static_property_name(node)
    }

    /// typescript-eslint's `getStaticMemberAccessValue`.
    fn member_name<'r, D: Doc>(
        node: &Node<'r, D>,
        value: &mut dyn FnMut(&Node<'r, D>) -> Value,
    ) -> Option<Vec<u16>> {
        key::member_access_name(node, value)
    }

    /// ESLint's `isSameReference`.
    fn same_reference<D: Doc>(
        left: &Node<'_, D>,
        right: &Node<'_, D>,
        static_keys: bool,
    ) -> Option<bool> {
        key::same_reference(left, right, static_keys)
    }

    fn feature(feature: ValueFeature<'_>) -> Result<(), Refusal> {
        match feature {
            ValueFeature::Type(name) => type_name(name, TYPESCRIPT_TYPES, "TypeScript"),
            ValueFeature::ToPrecision => Ok(()),
            ValueFeature::Compare(SemanticText::Structure(_), _) => Err(Refusal::Refused(
                "the structural modes compare Python and PHP syntax; use tokens for TypeScript"
                    .into(),
            )),
            ValueFeature::Compare(
                SemanticText::Value
                | SemanticText::Key
                | SemanticText::Member
                | SemanticText::Name
                | SemanticText::Reference { .. },
                _,
            ) => Ok(()),
        }
    }
}
