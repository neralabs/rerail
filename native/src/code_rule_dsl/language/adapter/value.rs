//! Value decoding and comparison (`value`, `compare`): `Semantic::Value`.

use super::{Blade, Ecma, Php, Python, Refusal};
use crate::code_rule_dsl::matcher::SemanticText;
use crate::code_rule_dsl::value::structure::{self, Equality, Tok};
use crate::code_rule_dsl::value::{self, Context, Lang, Value, key, php_compare, php_structure};
use ast_grep_core::{Doc, Node};

/// A DSL option of `value` or `compare` a family may refuse.
#[derive(Clone, Copy)]
pub(crate) enum ValueFeature<'a> {
    /// A `value` `type` name.
    Type(&'a str),
    /// A semantic `compare` text mode, and its name as the rule writes it.
    Compare(SemanticText, &'a str),
}

/// A family's values.
pub(crate) trait ValueRules {
    /// Whether the family decodes values: its languages' registry capability,
    /// which the adapter kit checks.
    #[cfg_attr(not(test), allow(dead_code))]
    const SUPPORTED: bool;
    /// The language whose semantics the family's values follow.
    const LANG: Lang;
    /// The grammar's expression kinds: only these have values.
    const EXPRESSION_KINDS: &'static [&'static str];
    /// The kinds whose value `decode` can know.
    const VALUE_KINDS: &'static [&'static str];
    /// The kinds of the names a reading following bindings gives a value (in PHP, named and
    /// class constants too).
    const NAME_KINDS: &'static [&'static str];
    /// Decodes `node` to its value; `Unknown` for anything that is not a literal the family knows.
    fn decode<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Value;
    /// A node's structure under one of the structural equalities; `None` where it is not known.
    fn structure<D: Doc>(
        node: &Node<'_, D>,
        equality: Equality,
        context: &mut Context<'_>,
    ) -> Option<Vec<Tok>>;
    /// A rule's JSON string as a value of the family's language.
    fn string(text: &str) -> Value;
    /// A rule's JSON number as a value of the family's language.
    fn number(number: &serde_json::Number) -> Value;
    /// Whether the node is the source its document spells, so a comparison of tokens may read it.
    fn spelled<D: Doc>(node: &Node<'_, D>) -> bool;
    /// What a node names (`compare`'s `name`); `None` for a node without one.
    fn node_name<D: Doc>(node: &Node<'_, D>) -> Option<Vec<u16>>;
    /// The static name of a key or of what holds one (`compare`'s `key`), `value` reading the
    /// nodes a computed name folds; `None` for a node without one.
    fn key_name<'r, D: Doc>(
        node: &Node<'r, D>,
        value: &mut dyn FnMut(&Node<'r, D>) -> Value,
    ) -> Option<Vec<u16>>;
    /// The name of a key or of what holds one, following bindings (`compare`'s `member`);
    /// `None` where it is not known.
    fn member_name<'r, D: Doc>(
        node: &Node<'r, D>,
        value: &mut dyn FnMut(&Node<'r, D>) -> Value,
    ) -> Option<Vec<u16>>;
    /// Whether two nodes are the same reference (`compare`'s `reference`); `None` where a
    /// literal does not decode.
    fn same_reference<D: Doc>(
        left: &Node<'_, D>,
        right: &Node<'_, D>,
        static_keys: bool,
    ) -> Option<bool>;
    /// Whether the family reads `feature`.
    fn feature(feature: ValueFeature<'_>) -> Result<(), Refusal>;
}

/// Type names `type` accepts: Python's `type(v).__name__`.
const PYTHON_TYPES: &[&str] = &[
    "int", "float", "complex", "str", "bytes", "bool", "NoneType", "ellipsis", "tuple", "list",
    "set", "dict",
];
/// Type names `type` accepts: TypeScript's `typeof` with `null` and `array` told apart from
/// `object`, and `regexp` within it.
const TYPESCRIPT_TYPES: &[&str] = &[
    "string",
    "number",
    "bigint",
    "boolean",
    "null",
    "undefined",
    "array",
    "object",
    "regexp",
];
/// PHP `get_debug_type` names Rerail can establish for constant expressions.
const PHP_TYPES: &[&str] = &["int", "float", "string", "bool", "null", "array"];

/// Whether `name` is one of `known`, the type names of `language`.
fn type_name(name: &str, known: &[&str], language: &str) -> Result<(), Refusal> {
    if known.contains(&name) {
        return Ok(());
    }
    Err(Refusal::Refused(
        format!(
            "`{name}` is not a {language} type name; expected one of {}",
            known.join(", ")
        )
        .into(),
    ))
}

impl ValueRules for Python {
    const SUPPORTED: bool = true;
    const LANG: Lang = Lang::Python;
    const EXPRESSION_KINDS: &'static [&'static str] = value::python::EXPRESSION_KINDS;
    const VALUE_KINDS: &'static [&'static str] = value::python::VALUE_KINDS;
    const NAME_KINDS: &'static [&'static str] = &["identifier"];

    fn decode<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Value {
        value::python::decode(node, context)
    }

    /// Ruff's equalities.
    fn structure<D: Doc>(
        node: &Node<'_, D>,
        equality: Equality,
        context: &mut Context<'_>,
    ) -> Option<Vec<Tok>> {
        structure::structure(node, equality, context)
    }

    fn string(text: &str) -> Value {
        Value::Str(text.to_owned())
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

impl ValueRules for Ecma {
    const SUPPORTED: bool = true;
    const LANG: Lang = Lang::Js;
    const EXPRESSION_KINDS: &'static [&'static str] = value::ecma::EXPRESSION_KINDS;
    const VALUE_KINDS: &'static [&'static str] = value::ecma::VALUE_KINDS;
    const NAME_KINDS: &'static [&'static str] = &["identifier", "undefined"];

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

    fn spelled<D: Doc>(_: &Node<'_, D>) -> bool {
        true
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

impl ValueRules for Php {
    const SUPPORTED: bool = true;
    const LANG: Lang = Lang::Php;
    const EXPRESSION_KINDS: &'static [&'static str] = value::php::EXPRESSION_KINDS;
    const VALUE_KINDS: &'static [&'static str] = value::php::VALUE_KINDS;
    const NAME_KINDS: &'static [&'static str] = &[
        "variable_name",
        "name",
        "qualified_name",
        "relative_name",
        "class_constant_access_expression",
    ];

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

    fn spelled<D: Doc>(_: &Node<'_, D>) -> bool {
        true
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

/// Blade's values are those of the PHP its template embeds (`blade::view`), but for the tokens
/// the view writes, which the template does not spell.
impl ValueRules for Blade {
    const SUPPORTED: bool = <Php as ValueRules>::SUPPORTED;
    const LANG: Lang = <Php as ValueRules>::LANG;
    const EXPRESSION_KINDS: &'static [&'static str] = <Php as ValueRules>::EXPRESSION_KINDS;
    const VALUE_KINDS: &'static [&'static str] = <Php as ValueRules>::VALUE_KINDS;
    const NAME_KINDS: &'static [&'static str] = <Php as ValueRules>::NAME_KINDS;

    fn decode<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Value {
        <Php as ValueRules>::decode(node, context)
    }

    /// The embedded PHP's; `compare` reads none where Blade wrote a token (`spelled`).
    fn structure<D: Doc>(
        node: &Node<'_, D>,
        equality: Equality,
        context: &mut Context<'_>,
    ) -> Option<Vec<Tok>> {
        <Php as ValueRules>::structure(node, equality, context)
    }

    fn string(text: &str) -> Value {
        <Php as ValueRules>::string(text)
    }

    fn number(number: &serde_json::Number) -> Value {
        <Php as ValueRules>::number(number)
    }

    /// Whether no token of the node is one the view writes (`@unless`'s `!(`), which a
    /// comparison of tokens must not read as the template's (`blade::rewritten`).
    fn spelled<D: Doc>(node: &Node<'_, D>) -> bool {
        !crate::code_rule_dsl::blade::rewritten(node)
    }

    fn node_name<D: Doc>(node: &Node<'_, D>) -> Option<Vec<u16>> {
        <Php as ValueRules>::node_name(node)
    }

    fn key_name<'r, D: Doc>(
        node: &Node<'r, D>,
        value: &mut dyn FnMut(&Node<'r, D>) -> Value,
    ) -> Option<Vec<u16>> {
        <Php as ValueRules>::key_name(node, value)
    }

    fn member_name<'r, D: Doc>(
        node: &Node<'r, D>,
        value: &mut dyn FnMut(&Node<'r, D>) -> Value,
    ) -> Option<Vec<u16>> {
        <Php as ValueRules>::member_name(node, value)
    }

    fn same_reference<D: Doc>(
        left: &Node<'_, D>,
        right: &Node<'_, D>,
        static_keys: bool,
    ) -> Option<bool> {
        <Php as ValueRules>::same_reference(left, right, static_keys)
    }

    fn feature(feature: ValueFeature<'_>) -> Result<(), Refusal> {
        match feature {
            ValueFeature::Compare(SemanticText::Structure(Equality::Parts), _) => Err(Refusal::Refused(
                "`structure-parts` compares Python's implicitly concatenated string parts; blade embeds PHP, which has no implicit string concatenation"
                    .into(),
            )),
            ValueFeature::Type(_) | ValueFeature::Compare(..) => {
                <Php as ValueRules>::feature(feature)
            }
        }
    }
}

#[cfg(test)]
pub(super) mod kit {
    use super::super::{Adapter, Refusal};
    use super::{ValueFeature, ValueRules};
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::matcher::SemanticText;
    use crate::code_rule_dsl::tape::TapeDoc;
    use crate::code_rule_dsl::value::structure::Equality;
    use crate::code_rule_dsl::value::{Context, Lang, Value};
    use ast_grep_core::AstGrep;

    /// What one family's values are.
    pub(in super::super) struct Expect {
        pub(in super::super) lang: Lang,
        /// The rule values of the JSON string `é` and the JSON numbers `16` and `2^64 - 1`.
        pub(in super::super) string: Value,
        pub(in super::super) numbers: [Value; 2],
        /// A document, the kind of the first node of that kind in its regions, and that node's
        /// value.
        pub(in super::super) decoded: (&'static str, &'static str, Value),
        pub(in super::super) name_kinds: &'static [&'static str],
        /// The verdict on the type name `str`, `Err` with its refusal's message.
        pub(in super::super) str_type: Result<(), &'static str>,
        /// The verdict on `compare`'s `structure-parts`.
        pub(in super::super) structure_parts: Result<(), &'static str>,
        /// A document and whether its regions' roots are spelled.
        pub(in super::super) spelled: (&'static str, bool),
        /// A document, the kind of the first node of that kind in its regions, and its name
        /// (`None`: `name` is refused).
        pub(in super::super) name: Option<(&'static str, &'static str, &'static str)>,
    }

    /// The message of a refusal, which `value` and `compare` only make `Refused`.
    fn refused(verdict: Result<(), Refusal>) -> Result<(), String> {
        verdict.map_err(|refusal| match refusal {
            Refusal::Refused(message) => message.into_owned(),
            Refusal::NotSupportedYet(message) => panic!("not supported yet: {message}"),
        })
    }

    /// The first node of `kind` in `source`'s regions, read by `with`.
    fn first<A: Adapter, T>(
        doc: &AstGrep<TapeDoc>,
        kind: &str,
        with: impl FnOnce(&ast_grep_core::Node<'_, TapeDoc>) -> T,
    ) -> T {
        let root = doc.root();
        let node = A::regions(&root)
            .find_map(|region| region.root.dfs().find(|node| node.kind() == kind))
            .unwrap_or_else(|| panic!("no {kind}"));
        with(&node)
    }

    pub(in super::super) fn check<A: Adapter>(
        expect: &Expect,
        dialect: GrammarDialect,
        tape: fn(&str, GrammarDialect) -> AstGrep<TapeDoc>,
    ) {
        assert_eq!(<A::Value as ValueRules>::LANG, expect.lang);
        assert_eq!(<A::Value as ValueRules>::string("é"), expect.string);
        assert_eq!(
            [
                <A::Value as ValueRules>::number(&16.into()),
                <A::Value as ValueRules>::number(&u64::MAX.into()),
            ],
            expect.numbers
        );
        let (source, kind, decoded) = &expect.decoded;
        assert!(<A::Value as ValueRules>::EXPRESSION_KINDS.contains(kind));
        assert!(<A::Value as ValueRules>::VALUE_KINDS.contains(kind));
        let doc = tape(source, dialect);
        let mut spend = || true;
        let value = first::<A, _>(&doc, kind, |node| {
            <A::Value as ValueRules>::decode(node, &mut Context::new(&mut spend))
        });
        assert_eq!(&value, decoded, "{source:?}");
        assert_eq!(<A::Value as ValueRules>::NAME_KINDS, expect.name_kinds);
        assert_eq!(
            refused(<A::Value as ValueRules>::feature(ValueFeature::Type("str"))),
            expect.str_type.map_err(str::to_owned)
        );
        assert_eq!(
            refused(<A::Value as ValueRules>::feature(ValueFeature::Compare(
                SemanticText::Structure(Equality::Parts),
                "structure-parts"
            ))),
            expect.structure_parts.map_err(str::to_owned)
        );
        let (source, spelled) = expect.spelled;
        let doc = tape(source, dialect);
        let root = doc.root();
        assert_eq!(
            A::regions(&root).all(|region| <A::Value as ValueRules>::spelled(&region.root)),
            spelled,
            "{source:?}"
        );
        if let Some((source, kind, name)) = expect.name {
            let doc = tape(source, dialect);
            let read = first::<A, _>(&doc, kind, <A::Value as ValueRules>::node_name);
            assert_eq!(read, Some(name.encode_utf16().collect()), "{source:?}");
        }
    }
}
