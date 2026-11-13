//! Java, read from tree-sitter-java's syntax alone. Its files parse and its structural terms run
//! (`docs/language-packs.md`, "The Java adapter", lists them); no semantic capability is read yet.
//! The registry marks each `not-supported-yet`, so a rule naming one, or a transform reading a
//! value (`toPrecision`, a `value` source), compiles to `language-not-supported` before any hook
//! below is asked; asked directly, the scope and value hooks answer the same. `Scope` and `Value`
//! are implemented here, reading nothing, because `Refused` stands in only for the concerns a
//! family may lack for good (arguments, calls, format arguments); each becomes Java's own module
//! as its capability is proven.

use super::QueryWhen;
use super::{
    Adapter, Refusal, Refused, ScopeFeature, ScopeRules, SyntaxFeature, SyntaxRules, ValueFeature,
    ValueRules,
};
use crate::code_rule_dsl::compile::config::Options;
use crate::code_rule_dsl::compile::options;
use crate::code_rule_dsl::language::Family;
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::matcher::{ScopeName, Unresolved};
use crate::code_rule_dsl::model::ViolationSpec;
use crate::code_rule_dsl::module_path::ModulePath;
use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
use crate::code_rule_dsl::scope::usage::{Steps, Usage};
use crate::code_rule_dsl::scope::{
    Binding, BuildError, Builder, Directives, Form, Lookup, Reading, Reference, ScopeTable,
};
use crate::code_rule_dsl::suppress::Suppress;
use crate::code_rule_dsl::value::structure::{Equality, Tok};
use crate::code_rule_dsl::value::{BigInt, Context, NodeRef, Value};
use ast_grep_core::{Doc, Node};
use std::borrow::Cow;
use std::cmp::Ordering;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

/// Java's family.
pub(crate) struct Java;

impl Adapter for Java {
    const FAMILY: Family = Family::Java;
    const LABEL: &'static str = "java";
    const TERMINATORS: Terminators = Terminators::Newlines;
    const PACKAGES: bool = false;
    const PACKAGE_MANIFESTS: bool = false;
    const JSX_RUNTIMES: bool = false;
    const DECLARATION_FILES: bool = false;
    const STRIPS_BOM: bool = false;
    const COMMENT_KINDS: &'static [&'static str] = &["line_comment", "block_comment"];
    const PREAMBLE_KINDS: &'static [&'static str] = &[];
    /// A `block` starts at its `{`, never at its first statement.
    const BODY_KINDS: &'static [&'static str] = &[];
    type Scope = Self;
    type Value = Self;
    type Arguments = Refused;
    type Calls = Refused;
    type FormatArgs = Refused;

    fn name(text: Cow<'_, str>) -> Cow<'_, str> {
        text
    }

    fn parse_input(source: &str) -> Cow<'_, str> {
        Cow::Borrowed(source)
    }

    fn module_path(_: &Path) -> Option<serde_json::Value> {
        None
    }

    /// No linter's suppression comments are read for Java yet (the registry lists none).
    fn suppresses<D: Doc>(
        _: &ViolationSpec,
        _: &Suppress,
        _: &Node<'_, D>,
        _: &str,
        _: &LineTable,
        _: Range<usize>,
        _: (usize, usize),
    ) -> bool {
        false
    }
}

/// The refusal reporting `message`.
fn refused(message: impl Into<Cow<'static, str>>) -> Result<(), Refusal> {
    Err(Refusal::Refused(message.into()))
}

/// What a hook of `capability`, which the registry marks `not-supported-yet`, answers.
fn not_supported_yet(capability: &str) -> Result<(), Refusal> {
    Err(Refusal::NotSupportedYet(Cow::Owned(format!(
        "the `{capability}` capability is not supported yet for java policies"
    ))))
}

/// Not read yet: a table binds and resolves nothing. The kinds are the ones the shared walks
/// name in every grammar.
impl ScopeRules for Java {
    const SUPPORTED: bool = false;
    const FLOW: bool = false;
    const STUBS: bool = false;
    const REDEFINES_SHADOWED: bool = false;
    const REFERENCE_KINDS: &'static [&'static str] = &[];
    const ANNOTATION_STRING_KINDS: &'static [&'static str] = &[];
    const NAME_KINDS: &'static [&'static str] = &[];
    const PAREN_KINDS: &'static [&'static str] = &["parenthesized_expression"];
    const DOTTED_NAME_KINDS: &'static [&'static str] = &[];
    const ATTRIBUTE_KINDS: &'static [&'static str] = &[];
    const THIS_KINDS: &'static [&'static str] = &["this"];
    const PRIVATE_NAME_KINDS: &'static [&'static str] = &[];
    const FORMS: &'static [(&'static str, Form)] = &[];
    const SCOPES: &'static [(&'static str, ScopeName)] = &[];
    const REFUSED_FILTERS: &'static [&'static str] = &[];
    const IMPORT_PATHS: bool = false;

    fn visit<D: Doc>(_: &mut Builder<'_>, _: &Node<'_, D>, _: Reading) -> Result<(), BuildError> {
        Ok(())
    }

    fn finish<D: Doc>(_: &mut ScopeTable, _: &Node<'_, D>) {}

    fn resolve<'t>(_: &'t ScopeTable, _: &Reference, _: &str) -> Option<&'t Binding> {
        None
    }

    fn binding<'t>(
        _: &'t ScopeTable,
        _: &Reference,
        _: &str,
    ) -> Option<(&'t Binding, Option<NodeRef>)> {
        None
    }

    fn binding_by<'t>(
        _: &'t ScopeTable,
        _: &Reference,
        _: &str,
        _: Lookup,
    ) -> Option<(&'t Binding, Option<NodeRef>)> {
        None
    }

    fn index(_: &ScopeTable, _: &Reference, _: &str, _: Lookup) -> Option<(u32, usize)> {
        None
    }

    /// Nothing is resolved, so no reference is decided.
    fn undecided(_: &ScopeTable, _: &Reference) -> bool {
        true
    }

    fn unbound_annotation(_: &ScopeTable, _: &Reference, _: &str) -> Option<(u32, usize)> {
        None
    }

    fn unresolved(
        _: &ScopeTable,
        _: &Reference,
        _: &str,
        _: Unresolved,
        _: &PythonOptions,
        _: Option<&LanguageOptions>,
        _: Option<&Directives>,
    ) -> bool {
        false
    }

    fn eslint_options(_: Option<&Arc<LanguageOptions>>) -> Option<Arc<LanguageOptions>> {
        None
    }

    fn ruff_options(_: Option<&Arc<PythonOptions>>) -> Option<Arc<PythonOptions>> {
        None
    }

    #[cfg(any(test, feature = "test-hooks"))]
    fn unbound_locally(_: &ScopeTable, _: &Reference, _: &str, _: bool) -> bool {
        false
    }

    fn usage<D: Doc>(
        table: &ScopeTable,
        _: &Node<'_, D>,
        steps: &mut Steps<'_>,
    ) -> Result<Usage, BuildError> {
        Ok(Usage::unread(table, steps))
    }

    fn unpackaged_module(_: &str) -> Option<ModulePath> {
        None
    }

    fn import_path<D: Doc>(_: &ScopeTable, _: &Node<'_, D>, _: Option<&str>) -> Option<String> {
        None
    }

    fn feature(_: ScopeFeature) -> Result<(), Refusal> {
        not_supported_yet("scope")
    }
}

/// Not read yet: every node's value is unknown, and no kind is an expression.
impl ValueRules for Java {
    const SUPPORTED: bool = false;
    const LANG: Family = Family::Java;
    const EXPRESSION_KINDS: &'static [&'static str] = &[];
    const VALUE_KINDS: &'static [&'static str] = &[];
    const NAME_KINDS: &'static [&'static str] = &[];

    fn truthy(_: &Value) -> Option<bool> {
        None
    }

    fn type_name(_: &Value) -> Option<&'static str> {
        None
    }

    fn equals(_: &Value, _: &Value) -> Option<bool> {
        None
    }

    fn order(_: &Value, _: &Value) -> Option<Option<Ordering>> {
        None
    }

    fn relation(_: &str, _: &Value, _: &Value) -> Option<bool> {
        None
    }

    fn numeric(_: &Value) -> bool {
        false
    }

    fn integer(_: &Value) -> Option<BigInt> {
        None
    }

    fn to_str<D: Doc>(_: &Value, _: &Node<'_, D>) -> Option<String> {
        None
    }

    fn decode<D: Doc>(_: &Node<'_, D>, _: &mut Context<'_>) -> Value {
        Value::Unknown
    }

    fn structure<D: Doc>(_: &Node<'_, D>, _: Equality, _: &mut Context<'_>) -> Option<Vec<Tok>> {
        None
    }

    /// Which Java type a rule's literal is, the value capability decides.
    fn string(_: &str) -> Value {
        Value::Unknown
    }

    fn number(_: &serde_json::Number) -> Value {
        Value::Unknown
    }

    fn spelled<D: Doc>(_: &Node<'_, D>) -> bool {
        true
    }

    fn node_name<D: Doc>(_: &Node<'_, D>) -> Option<Vec<u16>> {
        None
    }

    fn key_name<'r, D: Doc>(
        _: &Node<'r, D>,
        _: &mut dyn FnMut(&Node<'r, D>) -> Value,
    ) -> Option<Vec<u16>> {
        None
    }

    fn member_name<'r, D: Doc>(
        _: &Node<'r, D>,
        _: &mut dyn FnMut(&Node<'r, D>) -> Value,
    ) -> Option<Vec<u16>> {
        None
    }

    fn same_reference<D: Doc>(_: &Node<'_, D>, _: &Node<'_, D>, _: bool) -> Option<bool> {
        None
    }

    fn feature(_: ValueFeature<'_>) -> Result<(), Refusal> {
        not_supported_yet("value")
    }
}

/// Java hosts no literal grammar and has no `languageOptions`; `when` reads the document's
/// options.
impl SyntaxRules for Java {
    const DOLLAR_NAMES: bool = false;

    fn feature(feature: SyntaxFeature) -> Result<(), Refusal> {
        match feature {
            SyntaxFeature::LanguageOptions => {
                refused("`languageOptions` is not available for java policies")
            }
            SyntaxFeature::Parse(grammar) => {
                refused(format!("`{}` parses {}", grammar.name(), grammar.host()))
            }
        }
    }

    fn compile_language_options(
        _: &serde_json::Value,
        path: &str,
    ) -> napi::Result<super::syntax::QueryOptions> {
        Err(crate::code_rule_dsl::json::invalid(format!(
            "{path}: `languageOptions` is not available for java policies"
        )))
    }

    fn compile_when(
        value: &serde_json::Value,
        path: &str,
        _: Option<&LanguageOptions>,
        _: Option<&PythonOptions>,
        document: &Options,
    ) -> napi::Result<QueryWhen> {
        Ok((options::compile_option_when(value, path, document)?, None))
    }
}

#[cfg(test)]
mod tests {
    use crate::code_rule_dsl::CompiledCodeRuleDsl;
    use crate::code_rule_dsl::test_support::*;
    use serde_json::{Value, json};

    /// One Java code rule, `strict`, forbidding what the ast query `ast` (its language set to
    /// Java) matches.
    fn query(mut ast: Value) -> napi::Result<CompiledCodeRuleDsl> {
        ast["language"] = json!("java");
        compile_document_with(
            clause_document(
                json!({ "id": "c", "forbid": { "ast": ast }, "violation": violation() }),
            ),
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
    }

    /// One Java code rule, `strict`, forbidding `rule`.
    fn java(rule: Value) -> napi::Result<CompiledCodeRuleDsl> {
        query(json!({ "rule": rule }))
    }

    fn evaluate_query(ast: Value, source: &str) -> Value {
        let compiled = query(ast).unwrap_or_else(|error| panic!("{}", error.reason));
        evaluate_file(&compiled, "java", "src/A.java", source)
    }

    fn evaluate(rule: Value, source: &str) -> Value {
        evaluate_query(json!({ "rule": rule }), source)
    }

    const CONTROLLER: &str = "@RestController\nclass InvoiceController {\n    private final InvoiceRepository invoices;\n    private final InvoiceService service;\n\n    void log() {\n        System.out.println(invoices);\n    }\n}\n";

    #[test]
    fn structural_terms_match_java_syntax() {
        let field = json!({
            "kind": "field_declaration",
            "has": { "field": "type", "regex": "Repository$" },
            "inside": {
                "kind": "class_declaration",
                "stopBy": "end",
                "has": { "kind": "modifiers", "has": { "kind": "marker_annotation", "regex": "^@RestController$" } },
            },
        });
        let result = evaluate(field, CONTROLLER);
        assert_violations(&result, 1);
        assert_eq!(
            matched_texts(&result, CONTROLLER),
            ["private final InvoiceRepository invoices;"]
        );
        let call = evaluate(
            json!({ "pattern": "System.out.println($$$ARGS)" }),
            CONTROLLER,
        );
        assert_violations(&call, 1);
        assert_eq!(
            matched_texts(&call, CONTROLLER),
            ["System.out.println(invoices) ARGS=invoices"]
        );
        let methods = evaluate(
            json!({ "kind": "method_declaration", "count": { "has": { "kind": "method_invocation", "stopBy": "end" }, "min": 1 } }),
            CONTROLLER,
        );
        assert_violations(&methods, 1);
    }

    #[test]
    fn position_layout_and_utilities_match_java_syntax() {
        let source = "class A {\n    // leading\n    void f(int a, int b) {} // trailing\n}\n";
        let second = evaluate(json!({ "kind": "formal_parameter", "nthChild": 2 }), source);
        assert_eq!(matched_texts(&second, source), ["int b"]);
        let trailing = evaluate(
            json!({ "kind": "line_comment", "layout": { "before": { "to": "token", "breaks": { "eq": 0 } } } }),
            source,
        );
        assert_eq!(matched_texts(&trailing, source), ["// trailing"]);
        let method = evaluate_query(
            json!({ "utils": { "method": { "kind": "method_declaration" } }, "rule": { "matches": "method" } }),
            source,
        );
        assert_eq!(matched_texts(&method, source), ["void f(int a, int b) {}"]);
    }

    /// The registry marks every capability `not-supported-yet`: a rule naming one is kept, inactive,
    /// until the engine reads it.
    #[test]
    fn each_semantic_predicate_is_not_supported_yet() {
        let identifier = |predicate: Value| {
            let mut rule = json!({ "kind": "identifier" });
            rule.as_object_mut()
                .expect("an object")
                .extend(predicate.as_object().expect("an object").clone());
            rule
        };
        for (predicate, rule) in [
            (
                "resolves",
                identifier(json!({ "resolves": { "unresolved": true } })),
            ),
            ("binds", identifier(json!({ "binds": { "used": false } }))),
            ("value", identifier(json!({ "value": { "eq": 1 } }))),
            (
                "compare",
                json!({ "pattern": "$A == $B", "compare": { "left": "A", "op": "eq", "right": "B", "text": "tokens" } }),
            ),
            (
                "arguments",
                json!({ "kind": "method_invocation", "arguments": { "signature": ["a"], "bind": { "a": "A" } } }),
            ),
            (
                "calls",
                json!({ "kind": "method_declaration", "calls": { "names": ["a.b"] } }),
            ),
            (
                "formatArgs",
                json!({ "kind": "method_invocation", "formatArgs": { "style": "printf", "string": "S", "args": "A" } }),
            ),
        ] {
            let reason = rejection(java(rule));
            assert!(
                reason.starts_with("language-not-supported: ")
                    && reason.ends_with(&format!(
                        "`{predicate}` is not supported yet for java policies"
                    )),
                "{predicate}: {reason}"
            );
        }
        let util = query(json!({
            "utils": { "unbound": { "kind": "identifier", "resolves": { "unresolved": true } } },
            "rule": { "matches": "unbound" },
        }));
        assert_eq!(
            rejection(util),
            "language-not-supported: /clauses/0/forbid/ast/utils/unbound/resolves: `resolves` is not supported yet for java policies"
        );
    }

    /// A transform reading a value needs the value capability, as `value` and `compare` do:
    /// `toPrecision`, and any transform with a `value` source. One reading text runs.
    #[test]
    fn a_transform_reading_a_value_is_not_supported_yet() {
        let transformed = |trans: Value| {
            query(json!({ "rule": { "pattern": "f($A)" }, "transform": { "T": trans } }))
        };
        for (trans, at, predicate) in [
            (
                json!({ "toPrecision": { "source": "$A", "digits": 3 } }),
                "toPrecision",
                "toPrecision",
            ),
            (
                json!({ "substring": { "source": "$A", "value": true, "startChar": 1 } }),
                "substring/value",
                "value",
            ),
            (
                json!({ "replace": { "source": "$A", "value": true, "replace": "a", "by": "b" } }),
                "replace/value",
                "value",
            ),
            (
                json!({ "convert": { "source": "$A", "value": true, "toCase": "upperCase" } }),
                "convert/value",
                "value",
            ),
        ] {
            assert_eq!(
                rejection(transformed(trans)),
                format!(
                    "language-not-supported: /clauses/0/forbid/ast/transform/T/{at}: `{predicate}` is not supported yet for java policies"
                )
            );
        }
        for text in [
            json!({ "substring": { "source": "$A", "startChar": 1 } }),
            json!({ "substring": { "source": "$A", "value": false, "startChar": 1 } }),
        ] {
            assert!(transformed(text).is_ok());
        }
    }

    /// The scope and value hooks are reached only through the registry's gate, which refuses
    /// first; asked directly, they answer as the registry does.
    #[test]
    fn scope_and_value_hooks_are_not_supported_yet() {
        use super::{Java, Refusal, ScopeFeature, ScopeRules, ValueFeature, ValueRules};
        use crate::code_rule_dsl::matcher::SemanticText;
        for feature in [
            ScopeFeature::This,
            ScopeFeature::Occurrence,
            ScopeFeature::Every,
        ] {
            assert!(matches!(
                <Java as ScopeRules>::feature(feature),
                Err(Refusal::NotSupportedYet(_))
            ));
        }
        for feature in [
            ValueFeature::ToPrecision,
            ValueFeature::Type("int"),
            ValueFeature::Compare(SemanticText::Value, "value"),
        ] {
            assert!(matches!(
                <Java as ValueRules>::feature(feature),
                Err(Refusal::NotSupportedYet(_))
            ));
        }
    }

    /// The registry lists no suppression for Java: no linter's comments are read. A kind added
    /// to the table needs its spec here.
    #[test]
    fn every_suppression_kind_is_refused() {
        use crate::code_rule_dsl::suppress::Suppression;
        let kinds = Suppression::all();
        assert!(
            !kinds.is_empty(),
            "the engine registers its suppression kinds as it loads"
        );
        for kind in kinds.iter().map(|kind| kind.key()) {
            let spec = match kind {
                "eslint" => json!({ "rules": ["strict"] }),
                "noqa" => json!({ "codes": ["F401"] }),
                "nosec" => json!({ "codes": ["B101"] }),
                "oxlint" => json!({ "rules": ["no-console"] }),
                "biome" => json!({ "rules": ["lint/suspicious/noConsole"] }),
                "phpstan" => json!({ "identifiers": ["variable.undefined"] }),
                "psalm" => json!({ "issues": ["UndefinedVariable"] }),
                "phpcs" => json!({ "sniffs": ["Squiz.PHP.DiscouragedFunctions"] }),
                "clippy" => json!({ "lints": ["clippy::all"] }),
                "sqlfluff" => json!({ "rules": ["CV08"] }),
                other => panic!("no spec for the suppression kind {other}"),
            };
            let mut suppressed = violation();
            suppressed["suppress"] = json!({ kind: spec });
            let reason = rejection(compile_document_with(
                clause_document(json!({
                    "id": "c",
                    "forbid": { "ast": { "language": "java", "rule": { "kind": "identifier" } } },
                    "violation": suppressed,
                })),
                &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
            ));
            assert!(
                reason.starts_with(&format!("/clauses/0/violation/suppress/{kind}: "))
                    && reason.ends_with("; a java query has none"),
                "{reason}"
            );
        }
    }

    #[test]
    fn rerail_ignore_is_read_in_line_and_block_comments() {
        let call = json!({ "pattern": "System.out.println($$$ARGS)" });
        let source = "class A {\n    void f() {\n        System.out.println(1); // rerail-ignore strict\n        /* rerail-ignore strict */\n        System.out.println(2);\n        System.out.println(3);\n    }\n}\n";
        let result = evaluate(call, source);
        assert_violations(&result, 1);
        assert_eq!(
            matched_texts(&result, source),
            ["System.out.println(3) ARGS=3"]
        );
    }

    /// tree-sitter-java 0.23.5 does not parse a type-use annotation on a varargs type. The method
    /// that holds it is an error region; the rest of the file is still matched.
    #[test]
    fn a_file_that_parses_with_errors_is_matched_outside_them() {
        let source = "class A {\n    void f(@Nullable Object @Nullable ... args) {}\n\n    void g() {\n        System.out.println(1);\n    }\n}\n";
        let result = evaluate(json!({ "pattern": "System.out.println($$$ARGS)" }), source);
        assert_eq!(
            matched_texts(&result, source),
            ["System.out.println(1) ARGS=1"]
        );
        let failures = result["failures"].as_array().expect("failures");
        assert_eq!(failures.len(), 1, "{result}");
        assert_eq!(failures[0]["code"], "source-partial", "{result}");
    }
}
