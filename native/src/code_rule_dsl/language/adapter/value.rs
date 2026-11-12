//! Value decoding and comparison (`value`, `compare`): `Semantic::Value`.

use super::Refusal;
use crate::code_rule_dsl::language::Family;
use crate::code_rule_dsl::matcher::SemanticText;
use crate::code_rule_dsl::value::structure::{Equality, Tok};
use crate::code_rule_dsl::value::{self, BigInt, Context, Ordered, TypeNames, Value};
use ast_grep_core::{Doc, Node};
use std::cmp::Ordering;
// cratify: `lang_python` is rerail-lang-python's (it mounts it)
// cratify: `lang_ecma` is rerail-lang-ecma's (it mounts it)
// cratify: `lang_php` is rerail-lang-php's (it mounts it)

/// A DSL option of `value` or `compare` a family may refuse.
#[derive(Clone, Copy)]
pub enum ValueFeature<'a> {
    /// A `value` `type` name.
    Type(&'a str),
    /// A semantic `compare` text mode, and its name as the rule writes it.
    Compare(SemanticText, &'a str),
    /// `transform`'s `toPrecision`: a number laid out as ECMAScript's
    /// `Number.prototype.toPrecision` lays it out.
    ToPrecision,
}

/// A family's values.
pub trait ValueRules {
    /// Whether the family decodes values: its languages' registry capability,
    /// which the adapter kit checks.
    #[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
    const SUPPORTED: bool;
    /// The family whose value semantics a reading's values follow (`Reading::lang`): its own,
    /// or the one its documents embed (Blade's are PHP's). A new family's is `Self::FAMILY`'s
    /// variant (`const LANG: Family = Family::Rust;`) with the hooks below.
    const LANG: Family;
    /// The grammar's expression kinds: only these have values.
    const EXPRESSION_KINDS: &'static [&'static str];
    /// The kinds whose value `decode` can know.
    const VALUE_KINDS: &'static [&'static str];
    /// The kinds of the names a reading following bindings gives a value (in PHP, named and
    /// class constants too).
    const NAME_KINDS: &'static [&'static str];
    /// Truthiness (`Value::truthy`): Python's `bool(v)`, JavaScript's ToBoolean (objects and
    /// arrays are truthy even when empty), PHP's `(bool)`. `None` when it is not known.
    fn truthy(value: &Value) -> Option<bool>;
    /// The type's name (`value`'s `type`): Python's `type(v).__name__`, for JavaScript `typeof`
    /// refined with `null`, `array` and `regexp`, PHP's `get_debug_type`.
    fn type_name(value: &Value) -> Option<&'static str>;
    /// Equality in the language's own sense (`compare`'s `eq`): Python `==`, JavaScript
    /// `Object.is`, PHP `===`. `None` when it depends on elements that are not known.
    fn equals(left: &Value, right: &Value) -> Option<bool>;
    /// Order in the language's own sense: `Some(None)` when the two do not order, `None` when it
    /// is not known.
    fn order(left: &Value, right: &Value) -> Option<Option<Ordering>>;
    /// Whether `left operator right` holds for a relational `operator` (`<`, `<=`, `>`, `>=`), as
    /// the language's relational operators read it (`compare`'s orderings).
    fn relation(operator: &str, left: &Value, right: &Value) -> Option<bool>;
    /// Whether a value is a number a bound (`value`'s `gt`, `le`, ...) compares.
    fn numeric(value: &Value) -> bool;
    /// The integer a value's bits are (`value`'s `bitsAny`); `None` for a value without one.
    fn integer(value: &Value) -> Option<BigInt>;
    /// A node's value converted to a string as the language converts it (`transform`'s value
    /// text); `None` where it is not known or has no such conversion.
    fn to_str<D: Doc>(value: &Value, node: &Node<'_, D>) -> Option<String>;
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
    /// The tree `grammar` parses of a string value's bytes (PHP's `Value::PhpStr`), its nodes mapped to `whole`;
    /// `None` where the budget ran out. A family whose values hold no such string never reads one.
    fn parse_string_value(
        _bytes: Vec<u8>,
        _whole: std::ops::Range<usize>,
        _grammar: crate::code_rule_dsl::literal::Grammar,
        _spend: &mut dyn FnMut(usize) -> bool,
    ) -> Option<crate::code_rule_dsl::literal::Parsed> {
        None
    }
}

/// Type names `type` accepts: Python's `type(v).__name__`.
pub const PYTHON_TYPES: &[&str] = &[
    "int", "float", "complex", "str", "bytes", "bool", "NoneType", "ellipsis", "tuple", "list",
    "set", "dict",
];
/// Type names `type` accepts: TypeScript's `typeof` with `null` and `array` told apart from
/// `object`, and `regexp` within it.
pub const TYPESCRIPT_TYPES: &[&str] = &[
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
pub const PHP_TYPES: &[&str] = &["int", "float", "string", "bool", "null", "array"];

/// `toPrecision` refused for `language`, whose numbers are not laid out as ECMAScript's.
pub fn no_to_precision(language: &str) -> Result<(), Refusal> {
    Err(Refusal::Refused(
        format!(
            "`toPrecision` lays a number out as ECMAScript's Number.prototype.toPrecision; {language} numbers have no such layout"
        )
        .into(),
    ))
}

/// Whether `name` is one of `known`, the type names of `language`.
pub fn type_name(name: &str, known: &[&str], language: &str) -> Result<(), Refusal> {
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

#[cfg(any(test, feature = "test-hooks"))]
pub mod kit {
    use super::super::{Adapter, Refusal};
    use super::{ValueFeature, ValueRules};
    use crate::code_rule_dsl::language::Family;
    use crate::code_rule_dsl::matcher::SemanticText;
    use crate::code_rule_dsl::tape::TapeDoc;
    use crate::code_rule_dsl::value::structure::Equality;
    use crate::code_rule_dsl::value::{Context, Value};
    use ast_grep_core::AstGrep;

    /// What one family's values are.
    pub struct Expect {
        pub lang: Family,
        /// The rule values of the JSON string `é` and the JSON numbers `16` and `2^64 - 1`.
        pub string: Value,
        pub numbers: [Value; 2],
        /// A source, the kind of the first node of that kind in its document, and that node's
        /// value.
        pub decoded: (&'static str, &'static str, Value),
        pub name_kinds: &'static [&'static str],
        /// The verdict on the type name `str`, `Err` with its refusal's message.
        pub str_type: Result<(), &'static str>,
        /// The verdict on `compare`'s `structure-parts`.
        pub structure_parts: Result<(), &'static str>,
        /// A source and whether its document's root is spelled.
        pub spelled: (&'static str, bool),
        /// A source, the kind of the first node of that kind in its document, and its name
        /// (`None`: `name` is refused).
        pub name: Option<(&'static str, &'static str, &'static str)>,
    }

    /// The message of a refusal, which `value` and `compare` only make `Refused`.
    fn refused(verdict: Result<(), Refusal>) -> Result<(), String> {
        verdict.map_err(|refusal| match refusal {
            Refusal::Refused(message) => message.into_owned(),
            Refusal::NotSupportedYet(message) => panic!("not supported yet: {message}"),
        })
    }

    /// The first node of `kind` in a document, read by `with`.
    fn first<A: Adapter, T>(
        doc: &AstGrep<TapeDoc>,
        kind: &str,
        with: impl FnOnce(&ast_grep_core::Node<'_, TapeDoc>) -> T,
    ) -> T {
        let node = doc
            .root()
            .dfs()
            .find(|node| node.kind() == kind)
            .unwrap_or_else(|| panic!("no {kind}"));
        with(&node)
    }

    /// Checks `A`'s values against `expect`, over the documents its semantics read of each source
    /// (`tape`).
    pub fn check<A: Adapter>(
        expect: &Expect,
        tape: impl Fn(&str) -> AstGrep<TapeDoc>,
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
        let doc = tape(source);
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
        let doc = tape(source);
        assert_eq!(
            <A::Value as ValueRules>::spelled(&doc.root()),
            spelled,
            "{source:?}"
        );
        if let Some((source, kind, name)) = expect.name {
            let doc = tape(source);
            let read = first::<A, _>(&doc, kind, <A::Value as ValueRules>::node_name);
            assert_eq!(read, Some(name.encode_utf16().collect()), "{source:?}");
        }
    }
}
