//! The format strings' family (decision 0176): Python's `%` formats and `str.format` templates, the `percent-format`
//! and `brace-format` dialects, whose trees are those of Ruff's parsers as `literal::percent` and `literal::brace`
//! port them (`literal_tape`). A document of either is only ever a guest: a string's value, parsed where a rule asks
//! (`parsed(s, "percentFormat", r)`). It has no names, bindings or calls: scope, values, arguments, calls and format
//! arguments are refused (the registry's `refused`), and each refused hook answers "nothing known". An `error` node's
//! value, Ruff's message, is the core relations' (`core_rr::DialectFacts::node_literal`).

use super::QueryWhen;
use super::{
    Adapter, Refusal, Refused, ScopeFeature, ScopeRules, SyntaxFeature, SyntaxRules, ValueFeature, ValueRules,
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
use serde_json::Value as JsonValue;
use std::borrow::Cow;
use std::cmp::Ordering;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

/// Format strings.
pub struct Format;

fn refused(message: impl Into<Cow<'static, str>>) -> Result<(), Refusal> {
    Err(Refusal::Refused(message.into()))
}

impl Adapter for Format {
    const FAMILY: Family = Family::Format;
    const LABEL: &'static str = "percent-format";
    const TERMINATORS: Terminators = Terminators::Newlines;
    const PACKAGES: bool = false;
    const PACKAGE_MANIFESTS: bool = false;
    const JSX_RUNTIMES: bool = false;
    const DECLARATION_FILES: bool = false;
    const STRIPS_BOM: bool = false;
    /// A format string has no comments.
    const COMMENT_KINDS: &'static [&'static str] = &[];
    const PREAMBLE_KINDS: &'static [&'static str] = &[];
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

    fn module_path(_: &Path) -> Option<JsonValue> {
        None
    }

    /// A format string has no suppression comments.
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

impl ScopeRules for Format {
    const SUPPORTED: bool = false;
    const FLOW: bool = false;
    const STUBS: bool = false;
    const REDEFINES_SHADOWED: bool = false;
    const REFERENCE_KINDS: &'static [&'static str] = &[];
    const ANNOTATION_STRING_KINDS: &'static [&'static str] = &[];
    const NAME_KINDS: &'static [&'static str] = &[];
    const PRIVATE_NAME_KINDS: &'static [&'static str] = &[];
    const FORMS: &'static [(&'static str, Form)] = &[];
    const SCOPES: &'static [(&'static str, ScopeName)] = &[];
    const REFUSED_FILTERS: &'static [&'static str] = &[];
    const IMPORT_PATHS: bool = false;
    const PAREN_KINDS: &'static [&'static str] = &[];
    const DOTTED_NAME_KINDS: &'static [&'static str] = &[];
    const ATTRIBUTE_KINDS: &'static [&'static str] = &[];
    const THIS_KINDS: &'static [&'static str] = &[];

    fn visit<D: Doc>(_: &mut Builder<'_>, _: &Node<'_, D>, _: Reading) -> Result<(), BuildError> {
        Ok(())
    }

    fn finish<D: Doc>(_: &mut ScopeTable, _: &Node<'_, D>) {}

    fn resolve<'t>(_: &'t ScopeTable, _: &Reference, _: &str) -> Option<&'t Binding> {
        None
    }

    fn binding<'t>(_: &'t ScopeTable, _: &Reference, _: &str) -> Option<(&'t Binding, Option<NodeRef>)> {
        None
    }

    fn binding_by<'t>(_: &'t ScopeTable, _: &Reference, _: &str, _: Lookup) -> Option<(&'t Binding, Option<NodeRef>)> {
        None
    }

    fn index(_: &ScopeTable, _: &Reference, _: &str, _: Lookup) -> Option<(u32, usize)> {
        None
    }

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

    /// Never reached: scope is refused, so no rule asks for a format string's usage.
    fn usage<D: Doc>(_: &ScopeTable, _: &Node<'_, D>, _: &mut Steps<'_>) -> Result<Usage, BuildError> {
        Err(BuildError::Budget)
    }

    fn unpackaged_module(_: &str) -> Option<ModulePath> {
        None
    }

    fn import_path<D: Doc>(_: &ScopeTable, _: &Node<'_, D>, _: Option<&str>) -> Option<String> {
        None
    }

    fn feature(_: ScopeFeature) -> Result<(), Refusal> {
        refused("scope is not available for format strings")
    }
}

impl ValueRules for Format {
    const SUPPORTED: bool = false;
    const LANG: Family = Family::Format;
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

    fn string(_: &str) -> Value {
        Value::Unknown
    }

    fn number(_: &serde_json::Number) -> Value {
        Value::Unknown
    }

    fn spelled<D: Doc>(_: &Node<'_, D>) -> bool {
        false
    }

    fn node_name<D: Doc>(_: &Node<'_, D>) -> Option<Vec<u16>> {
        None
    }

    fn key_name<'r, D: Doc>(_: &Node<'r, D>, _: &mut dyn FnMut(&Node<'r, D>) -> Value) -> Option<Vec<u16>> {
        None
    }

    fn member_name<'r, D: Doc>(_: &Node<'r, D>, _: &mut dyn FnMut(&Node<'r, D>) -> Value) -> Option<Vec<u16>> {
        None
    }

    fn same_reference<D: Doc>(_: &Node<'_, D>, _: &Node<'_, D>, _: bool) -> Option<bool> {
        None
    }

    fn feature(_: ValueFeature<'_>) -> Result<(), Refusal> {
        refused("values are not available for format strings")
    }
}

/// A format string hosts no literal grammar a policy parses, and has no `languageOptions`.
impl SyntaxRules for Format {
    const DOLLAR_NAMES: bool = false;

    fn feature(feature: SyntaxFeature) -> Result<(), Refusal> {
        match feature {
            SyntaxFeature::LanguageOptions => refused("`languageOptions` is not available for format strings"),
            SyntaxFeature::Parse(grammar) => refused(format!("`{}` parses {}", grammar.name(), grammar.host())),
        }
    }

    fn compile_language_options(_: &JsonValue, _: &str) -> napi::Result<(Option<Arc<LanguageOptions>>, Option<Arc<PythonOptions>>)> {
        Ok((None, None))
    }

    fn compile_when(
        value: &JsonValue,
        path: &str,
        _: Option<&LanguageOptions>,
        _: Option<&PythonOptions>,
        document: &Options,
    ) -> napi::Result<QueryWhen> {
        Ok((options::compile_option_when(value, path, document)?, None))
    }
}
