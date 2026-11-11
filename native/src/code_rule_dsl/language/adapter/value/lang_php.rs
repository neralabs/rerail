//! Moved from `value.rs` by the crate split (tools/cratify/moves.tsv).

use super::*;

impl ValueRules for Php {
    const SUPPORTED: bool = true;
    const LANG: Family = Family::Php;
    const EXPRESSION_KINDS: &'static [&'static str] = value::php::EXPRESSION_KINDS;
    const VALUE_KINDS: &'static [&'static str] = value::php::VALUE_KINDS;
    const NAME_KINDS: &'static [&'static str] = &[
        "variable_name",
        "name",
        "qualified_name",
        "relative_name",
        "class_constant_access_expression",
    ];

    fn truthy(value: &Value) -> Option<bool> {
        value::php::truthy(value)
    }

    fn type_name(value: &Value) -> Option<&'static str> {
        value::php::type_name(value)
    }

    fn equals(left: &Value, right: &Value) -> Option<bool> {
        value::php::identical(left, right)
    }

    /// As PHP 8's `<=>`.
    fn order(left: &Value, right: &Value) -> Option<Option<Ordering>> {
        value::php::order(left, right)
    }

    /// As PHP's relational operators: `a > b` is `b < a`, which PHP's comparison of arrays does
    /// not make the same question.
    fn relation(operator: &str, left: &Value, right: &Value) -> Option<bool> {
        value::php::relation(operator, left, right)
    }

    fn numeric(value: &Value) -> bool {
        matches!(value, Value::Int(_) | Value::Float(_) | Value::BigInt(_))
    }

    fn integer(value: &Value) -> Option<BigInt> {
        match value {
            Value::Int(value) => Some(value.clone()),
            Value::BigInt(value) => Some(BigInt::from(*value)),
            _ => None,
        }
    }

    fn to_str<D: Doc>(value: &Value, _: &Node<'_, D>) -> Option<String> {
        value::php::to_string(value)
    }

    fn decode<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Value {
        value::php::decode(node, context)
    }

    /// PHP-Parser's trees.
    fn structure<D: Doc>(
        node: &Node<'_, D>,
        equality: Equality,
        context: &mut Context<'_>,
    ) -> Option<Vec<Tok>> {
        php_structure::structure(node, equality, context)
    }

    /// UTF-8 bytes.
    fn string(text: &str) -> Value {
        Value::PhpStr(text.as_bytes().to_vec())
    }

    /// An `int` within 64 bits, a `float` past them.
    fn number(number: &serde_json::Number) -> Value {
        match number.as_i64() {
            Some(integer) => Value::int(integer),
            None => Value::Float(number.as_f64().unwrap_or(f64::NAN)),
        }
    }

    /// Whether no token of the node is one an embedder wrote (a Blade view's `@unless` `!(`),
    /// which a comparison of tokens must not read as the template's (`embed::rewritten`).
    fn spelled<D: Doc>(node: &Node<'_, D>) -> bool {
        !crate::code_rule_dsl::origin::rewritten(node)
    }

    /// The name PHP looks a symbol up by, or a string literal's value.
    fn node_name<D: Doc>(node: &Node<'_, D>) -> Option<Vec<u16>> {
        php_compare::node_name(node)
    }

    fn key_name<'r, D: Doc>(
        node: &Node<'r, D>,
        value: &mut dyn FnMut(&Node<'r, D>) -> Value,
    ) -> Option<Vec<u16>> {
        php_compare::key_name(node, value).flatten()
    }

    /// The key's name, a computed name's variables followed: a node holding no key has no
    /// member name either.
    fn member_name<'r, D: Doc>(
        node: &Node<'r, D>,
        value: &mut dyn FnMut(&Node<'r, D>) -> Value,
    ) -> Option<Vec<u16>> {
        php_compare::key_name(node, value).flatten()
    }

    fn same_reference<D: Doc>(
        left: &Node<'_, D>,
        right: &Node<'_, D>,
        static_keys: bool,
    ) -> Option<bool> {
        php_compare::same_reference(left, right, static_keys)
    }

    fn feature(feature: ValueFeature<'_>) -> Result<(), Refusal> {
        match feature {
            ValueFeature::Type(name) => type_name(name, PHP_TYPES, "PHP"),
            ValueFeature::ToPrecision => no_to_precision("PHP"),
            ValueFeature::Compare(SemanticText::Structure(Equality::Parts), _) => Err(Refusal::Refused(
                "`structure-parts` compares Python's implicitly concatenated string parts; PHP has no implicit string concatenation"
                    .into(),
            )),
            ValueFeature::Compare(
                SemanticText::Value
                | SemanticText::Key
                | SemanticText::Member
                | SemanticText::Name
                | SemanticText::Reference { .. }
                | SemanticText::Structure(Equality::Plain | Equality::Hash),
                _,
            ) => Ok(()),
        }
    }
}
